use std::collections::{BTreeMap, BTreeSet};

use ckg_domain::{
    CanonicalId, ChannelDirection, ConsumedContract, ContractBatch, EvidenceKind, GraphEdge,
    Mechanism, ProvidedEndpoint, RelationKind, resource_id, topic_id,
};
use ckg_graph_delta::{DeltaOp, GraphDelta};
use ckg_repository_config::WorkspaceConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::link_set::{LinkEdge, LinkSet, ResourceState, TopicState, link_provenance};
use crate::resource::{resolve_resource, resource_edge_kind};
use crate::route::{is_generic, normalize_route, routes_match};

pub(crate) const RANK_LOW: u8 = 0;
pub(crate) const RANK_MEDIUM: u8 = 1;
pub(crate) const RANK_HIGH: u8 = 2;

const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// One indexed branch head participating in the link pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchHead {
    /// Repository config id (matches `base_urls` / `target_repo` values).
    pub repo_id: String,
    /// GitHub slug (or fallback id) used for canonical ids.
    pub github: String,
    pub branch: String,
    pub sha: String,
    pub commit_id: CanonicalId,
}

impl BranchHead {
    pub fn label(&self) -> String {
        format!("{}@{}", self.repo_id, self.branch)
    }
}

fn rank_str(rank: u8) -> &'static str {
    match rank {
        RANK_LOW => "low",
        RANK_MEDIUM => "medium",
        _ => "high",
    }
}

fn rank_of(props: &Map<String, Value>) -> u8 {
    match props.get("confidence").and_then(Value::as_str) {
        Some("low") => RANK_LOW,
        Some("high") => RANK_HIGH,
        _ => RANK_MEDIUM,
    }
}

fn evidence_of(props: &Map<String, Value>) -> Vec<(String, String)> {
    props
        .get("evidence")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let obj = entry.as_object()?;
                    Some((
                        obj.get("kind")?.as_str()?.to_string(),
                        obj.get("detail")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn set_evidence(props: &mut Map<String, Value>, entries: Vec<(String, String)>) {
    let array = entries
        .into_iter()
        .map(|(kind, detail)| json!({ "kind": kind, "detail": detail }))
        .collect::<Vec<Value>>();
    props.insert("evidence".to_string(), Value::Array(array));
}

fn push_evidence(entries: &mut Vec<(String, String)>, kind: &str, detail: &str) {
    let entry = (kind.to_string(), detail.to_string());
    if !entries.contains(&entry) {
        entries.push(entry);
    }
}

/// Merge `incoming` into `existing`: union `branch_pairs`, union deduped
/// evidence, keep the properties of the higher-confidence side.
fn merge_link(existing: &mut LinkEdge, incoming: LinkEdge) {
    let pairs: BTreeSet<String> = existing
        .branch_pairs
        .iter()
        .cloned()
        .chain(incoming.branch_pairs.iter().cloned())
        .collect();
    let mut ev = evidence_of(&existing.properties);
    for entry in evidence_of(&incoming.properties) {
        if !ev.contains(&entry) {
            ev.push(entry);
        }
    }
    if rank_of(&incoming.properties) > rank_of(&existing.properties) {
        existing.properties = incoming.properties;
    }
    set_evidence(&mut existing.properties, ev);
    existing.branch_pairs = pairs.into_iter().collect();
}

fn upsert_link(map: &mut BTreeMap<(String, String, String), LinkEdge>, edge: LinkEdge) {
    let key = edge.key();
    match map.get_mut(&key) {
        Some(existing) => merge_link(existing, edge),
        None => {
            map.insert(key, edge);
        }
    }
}

fn base_url_repo(base_urls: &BTreeMap<String, String>, hint: &str) -> Option<String> {
    let hint = hint.trim().to_ascii_lowercase();
    if hint.is_empty() {
        return None;
    }
    let mut best: Option<(usize, String)> = None;
    for (base, repo) in base_urls {
        let base = base.trim().to_ascii_lowercase();
        if base.is_empty() || !hint.starts_with(&base) {
            continue;
        }
        let boundary_ok = match hint.as_bytes().get(base.len()) {
            None => true,
            Some(c) => matches!(c, b'/' | b'?' | b'#' | b':'),
        };
        if !boundary_ok {
            continue;
        }
        if best.as_ref().is_none_or(|(len, _)| base.len() > *len) {
            best = Some((base.len(), repo.clone()));
        }
    }
    best.map(|(_, repo)| repo)
}

fn evidence_route_candidate(value: &str) -> &str {
    let value = value.trim();
    if let Some((first, rest)) = value.split_once(' ') {
        let first = first.to_ascii_lowercase();
        if HTTP_METHODS.contains(&first.as_str()) {
            return rest.trim();
        }
    }
    value
}

/// Does an evidence value (URL, `METHOD /route`, or bare route) match the
/// consumed route?
fn evidence_matches_route(value: &str, route: &str) -> bool {
    if route.trim().is_empty() {
        return false;
    }
    let candidate = evidence_route_candidate(value);
    let path = match candidate.find("://") {
        Some(idx) => {
            let after = &candidate[idx + 3..];
            after.find('/').map(|i| &after[i..]).unwrap_or("")
        }
        None => candidate,
    };
    if path.is_empty() {
        return false;
    }
    routes_match(path, route)
}

struct Restriction {
    repos: Option<BTreeSet<String>>,
    evidence: Vec<(String, String)>,
}

fn restriction_for(
    workspace: &WorkspaceConfig,
    batch: &ContractBatch,
    consume: &ConsumedContract,
) -> Restriction {
    let mut constraints: Vec<BTreeSet<String>> = Vec::new();
    let mut evidence: Vec<(String, String)> = Vec::new();

    if let Some(repo) = base_url_repo(&workspace.links.base_urls, &consume.target_hint) {
        let mut set = BTreeSet::new();
        set.insert(repo);
        constraints.push(set);
        push_evidence(&mut evidence, "target_hint", consume.target_hint.trim());
    }

    for record in &batch.evidence {
        let kind_str = match record.kind {
            EvidenceKind::Openapi => "openapi",
            EvidenceKind::Config => "config",
            _ => continue,
        };
        let target = record.target_repo.trim();
        if target.is_empty() || !evidence_matches_route(&record.value, &consume.route) {
            continue;
        }
        let mut set = BTreeSet::new();
        set.insert(target.to_string());
        constraints.push(set);
        push_evidence(&mut evidence, kind_str, &record.value);
    }

    let repos = if constraints.is_empty() {
        None
    } else {
        let mut intersection = constraints[0].clone();
        for set in constraints.iter().skip(1) {
            intersection = intersection.intersection(set).cloned().collect();
        }
        if intersection.is_empty() {
            intersection = constraints[0].clone();
        }
        Some(intersection)
    };
    Restriction { repos, evidence }
}

fn rest_route_ok(route: &str) -> bool {
    !route.trim().is_empty() && !is_generic(route)
}

fn optional_route_ok(route: &str) -> bool {
    route.trim().is_empty() || !is_generic(route)
}

/// `.`-boundary suffix relation: `orders.v1.OrderService` matches
/// `v1.OrderService` but `rders.v1.OrderService` does not.
fn service_boundary_match(a: &str, b: &str) -> bool {
    let (long, short) = if a.len() >= b.len() { (a, b) } else { (b, a) };
    long.len() > short.len()
        && long.ends_with(short)
        && long.as_bytes()[long.len() - short.len() - 1] == b'.'
}

fn grpc_matches(consume: &ConsumedContract, provide: &ProvidedEndpoint) -> bool {
    if consume.method.is_empty() || provide.method.is_empty() {
        return false;
    }
    if consume.method != provide.method {
        return false;
    }
    if consume.service.is_empty() || provide.service.is_empty() {
        return true;
    }
    consume.service == provide.service || service_boundary_match(&consume.service, &provide.service)
}

fn jsonrpc_matches(consume: &ConsumedContract, provide: &ProvidedEndpoint) -> bool {
    if consume.method.is_empty() || consume.method != provide.method {
        return false;
    }
    if consume.route.is_empty() || provide.route.is_empty() {
        return true;
    }
    normalize_route(&consume.route) == normalize_route(&provide.route)
}

fn provide_matches(consume: &ConsumedContract, provide: &ProvidedEndpoint) -> bool {
    if consume.mechanism != provide.mechanism || consume.mechanism.is_messaging() {
        return false;
    }
    match consume.mechanism {
        Mechanism::Rest => {
            rest_route_ok(&consume.route)
                && rest_route_ok(&provide.route)
                && routes_match(&consume.route, &provide.route)
        }
        Mechanism::Grpc => grpc_matches(consume, provide),
        Mechanism::JsonRpc => {
            optional_route_ok(&consume.route)
                && optional_route_ok(&provide.route)
                && jsonrpc_matches(consume, provide)
        }
        Mechanism::WebSocket | Mechanism::Sse => {
            rest_route_ok(&consume.route)
                && rest_route_ok(&provide.route)
                && routes_match(&consume.route, &provide.route)
        }
        _ => false,
    }
}

/// Mechanism-specific confidence ceiling (the final confidence is the
/// minimum of this and the candidate-level confidence).
fn mechanism_rank(consume: &ConsumedContract, provide: &ProvidedEndpoint) -> u8 {
    match consume.mechanism {
        Mechanism::Grpc => {
            if consume.service.is_empty() || provide.service.is_empty() {
                RANK_LOW
            } else {
                RANK_MEDIUM
            }
        }
        _ => RANK_HIGH,
    }
}

fn baseline_evidence(consume: &ConsumedContract) -> (&'static str, String) {
    match consume.mechanism {
        Mechanism::Grpc => {
            let detail = if consume.service.is_empty() {
                consume.method.clone()
            } else {
                format!("{}/{}", consume.service, consume.method)
            };
            ("rpc", detail)
        }
        _ => ("route", normalize_route(&consume.route)),
    }
}

fn mechanism_fields(
    consume: &ConsumedContract,
    provide: &ProvidedEndpoint,
    props: &mut Map<String, Value>,
) {
    match consume.mechanism {
        Mechanism::Rest => {
            let method = if consume.http_method.trim().is_empty() {
                provide.http_method.trim().to_string()
            } else {
                consume.http_method.trim().to_string()
            };
            if !method.is_empty() {
                props.insert(
                    "http_method".to_string(),
                    json!(method.to_ascii_uppercase()),
                );
            }
            props.insert("route".to_string(), json!(normalize_route(&consume.route)));
        }
        Mechanism::JsonRpc => {
            props.insert("method".to_string(), json!(consume.method));
            props.insert("route".to_string(), json!(normalize_route(&consume.route)));
        }
        Mechanism::WebSocket | Mechanism::Sse => {
            props.insert("route".to_string(), json!(normalize_route(&consume.route)));
        }
        Mechanism::Grpc => {
            let service = if consume.service.is_empty() {
                provide.service.clone()
            } else {
                consume.service.clone()
            };
            props.insert("service".to_string(), json!(service));
            props.insert("method".to_string(), json!(consume.method));
        }
        _ => {}
    }
}

fn pair_label(consumer: &BranchHead, provider: &BranchHead) -> String {
    format!("{} -> {}", consumer.label(), provider.label())
}

/// Identity for an unmatched REST/WS/SSE consume: the hint's host when
/// present, otherwise the normalized (non-generic) route.
fn api_fallback_identity(consume: &ConsumedContract) -> Option<String> {
    let hint = consume.target_hint.trim();
    if !hint.is_empty() {
        let low = hint.to_ascii_lowercase();
        if let Some(after) = low
            .strip_prefix("http://")
            .or_else(|| low.strip_prefix("https://"))
        {
            let host = after.split(['/', '?', '#']).next().unwrap_or("");
            if !host.is_empty() {
                return Some(host.to_string());
            }
        } else {
            let host = hint
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !host.is_empty() {
                return Some(host);
            }
        }
    }
    let route = consume.route.trim();
    if !route.is_empty() && !is_generic(route) {
        return Some(normalize_route(route));
    }
    None
}

/// Unmatched REST/WS/SSE consumes still depend on an external API: emit
/// `CallsApi -> Resource(api)` instead of dropping them.
fn emit_external_api(
    consume: &ConsumedContract,
    consumer: &BranchHead,
    resources: &mut BTreeMap<String, ResourceState>,
    edges: &mut BTreeMap<(String, String, String), LinkEdge>,
) {
    if !matches!(
        consume.mechanism,
        Mechanism::Rest | Mechanism::WebSocket | Mechanism::Sse
    ) || consume.caller_id.as_str().is_empty()
    {
        return;
    }
    let Some(identity) = api_fallback_identity(consume) else {
        return;
    };
    let id = resource_id("api", &identity);
    let entry = resources
        .entry(id.to_string())
        .or_insert_with(|| ResourceState {
            id: id.clone(),
            resource_type: "api".to_string(),
            identity: identity.clone(),
            mechanism: consume.mechanism.as_str().to_string(),
            commits: Vec::new(),
        });
    let commit = consumer.commit_id.to_string();
    if !entry.commits.contains(&commit) {
        entry.commits.push(commit);
        entry.commits.sort();
    }
    let mut props = Map::new();
    props.insert("mechanism".to_string(), json!(consume.mechanism.as_str()));
    props.insert("confidence".to_string(), json!(rank_str(RANK_LOW)));
    set_evidence(&mut props, vec![("unmatched".to_string(), identity)]);
    props.insert("resource_type".to_string(), json!("api"));
    if !consume.route.trim().is_empty() {
        props.insert("route".to_string(), json!(normalize_route(&consume.route)));
    }
    upsert_link(
        edges,
        LinkEdge {
            kind: RelationKind::CallsApi,
            from: consume.caller_id.clone(),
            to: id,
            properties: props,
            branch_pairs: vec![consumer.label()],
        },
    );
}

fn match_service_links(
    workspace: &WorkspaceConfig,
    inputs: &[(BranchHead, ContractBatch)],
    resources: &mut BTreeMap<String, ResourceState>,
    edges: &mut BTreeMap<(String, String, String), LinkEdge>,
) {
    for (ci, (consumer, cbatch)) in inputs.iter().enumerate() {
        for consume in &cbatch.consumes {
            if consume.mechanism.is_messaging() {
                continue;
            }
            let restriction = restriction_for(workspace, cbatch, consume);

            let mut candidates: Vec<(usize, Vec<&ProvidedEndpoint>)> = Vec::new();
            let mut candidate_repos: BTreeSet<String> = BTreeSet::new();
            for (pi, (provider, pbatch)) in inputs.iter().enumerate() {
                if pi == ci {
                    continue;
                }
                if let Some(repos) = &restriction.repos
                    && !repos.contains(&provider.repo_id)
                {
                    continue;
                }
                let matched: Vec<&ProvidedEndpoint> = pbatch
                    .provides
                    .iter()
                    .filter(|provide| provide_matches(consume, provide))
                    .collect();
                if matched.is_empty() {
                    continue;
                }
                candidate_repos.insert(provider.repo_id.clone());
                candidates.push((pi, matched));
            }

            if candidate_repos.is_empty() {
                emit_external_api(consume, consumer, resources, edges);
                continue;
            }

            let base_rank = if candidate_repos.len() > 1 {
                RANK_LOW
            } else if restriction.repos.is_some() {
                RANK_HIGH
            } else {
                RANK_MEDIUM
            };

            let (baseline_kind, baseline_detail) = baseline_evidence(consume);
            let mut evidence: Vec<(String, String)> = Vec::new();
            push_evidence(&mut evidence, baseline_kind, &baseline_detail);
            for entry in &restriction.evidence {
                push_evidence(&mut evidence, &entry.0, &entry.1);
            }
            if candidate_repos.len() > 1 {
                push_evidence(
                    &mut evidence,
                    "candidates",
                    &candidate_repos
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }

            for (pi, provides) in &candidates {
                let provider = &inputs[*pi].0;
                for provide in provides {
                    if consume.caller_id.as_str().is_empty()
                        || provide.handler_id.as_str().is_empty()
                    {
                        continue;
                    }
                    let rank = base_rank.min(mechanism_rank(consume, provide));
                    let mut props = Map::new();
                    props.insert("mechanism".to_string(), json!(consume.mechanism.as_str()));
                    props.insert("confidence".to_string(), json!(rank_str(rank)));
                    set_evidence(&mut props, evidence.clone());
                    mechanism_fields(consume, provide, &mut props);
                    upsert_link(
                        edges,
                        LinkEdge {
                            kind: RelationKind::CallsApi,
                            from: consume.caller_id.clone(),
                            to: provide.handler_id.clone(),
                            properties: props,
                            branch_pairs: vec![pair_label(consumer, provider)],
                        },
                    );
                }
            }
        }
    }
}

fn unresolved_template(channel: &str) -> bool {
    // Any placeholder (`${VAR}`, `$(VAR)`, `{{var}}`) cannot be resolved to a
    // concrete hub at link time; skip rather than mint a garbage topic.
    channel.contains("${") || channel.contains("$(") || channel.contains("{{")
}

fn resolve_broker(
    workspace: &WorkspaceConfig,
    channel: &str,
    record_broker: &str,
) -> (String, bool) {
    if let Some(broker) = workspace.links.channel_brokers.get(channel) {
        return (broker.trim().to_ascii_lowercase(), true);
    }
    let lower = channel.to_ascii_lowercase();
    if let Some((_, broker)) = workspace
        .links
        .channel_brokers
        .iter()
        .find(|(key, _)| key.to_ascii_lowercase() == lower)
    {
        return (broker.trim().to_ascii_lowercase(), true);
    }
    (record_broker.trim().to_ascii_lowercase(), false)
}

fn infer_channel_type(broker: &str) -> String {
    let broker = broker.to_ascii_lowercase();
    if broker.contains("rabbit") || broker.contains("amqp") || broker.contains("sqs") {
        "queue".to_string()
    } else {
        "topic".to_string()
    }
}

fn mechanism_str(broker: &str) -> String {
    match Mechanism::from_broker(broker) {
        Some(mechanism) => mechanism.as_str().to_string(),
        None => broker.trim().to_ascii_lowercase(),
    }
}

fn match_channel_links(
    workspace: &WorkspaceConfig,
    inputs: &[(BranchHead, ContractBatch)],
    topics: &mut BTreeMap<String, TopicState>,
    edges: &mut BTreeMap<(String, String, String), LinkEdge>,
) {
    for (head, batch) in inputs {
        for channel_ref in &batch.channels {
            let channel = channel_ref.channel.trim();
            if channel.is_empty() || unresolved_template(channel) {
                continue;
            }
            let (broker, from_config) =
                resolve_broker(workspace, channel, channel_ref.broker.trim());
            let channel_type = if !channel_ref.channel_type.trim().is_empty() {
                channel_ref.channel_type.trim().to_string()
            } else {
                infer_channel_type(&broker)
            };
            let id = topic_id(&broker, channel);
            let entry = topics.entry(id.to_string()).or_insert_with(|| TopicState {
                id: id.clone(),
                broker: broker.clone(),
                channel: channel.to_string(),
                channel_type: channel_type.clone(),
                commits: Vec::new(),
            });
            let commit = head.commit_id.to_string();
            if !entry.commits.contains(&commit) {
                entry.commits.push(commit);
                entry.commits.sort();
            }

            if channel_ref.node_id.as_str().is_empty() {
                continue;
            }
            let kind = match channel_ref.direction {
                ChannelDirection::Publish => RelationKind::PublishTo,
                ChannelDirection::Subscribe => RelationKind::ConsumeFrom,
            };
            let rank = if from_config { RANK_HIGH } else { RANK_MEDIUM };
            let mut props = Map::new();
            props.insert("mechanism".to_string(), json!(mechanism_str(&broker)));
            props.insert("confidence".to_string(), json!(rank_str(rank)));
            set_evidence(
                &mut props,
                vec![("channel".to_string(), channel.to_string())],
            );
            props.insert("broker".to_string(), json!(broker));
            props.insert("channel".to_string(), json!(channel));
            props.insert("channel_type".to_string(), json!(channel_type));
            upsert_link(
                edges,
                LinkEdge {
                    kind,
                    from: channel_ref.node_id.clone(),
                    to: id.clone(),
                    properties: props,
                    branch_pairs: vec![head.label()],
                },
            );
        }
    }
}

/// Extracted external-resource references → resource nodes + fine-grained
/// dependency edges (ReadsFrom / WritesTo / Invokes / ConnectsTo / Queries).
fn match_resource_links(
    workspace: &WorkspaceConfig,
    inputs: &[(BranchHead, ContractBatch)],
    resources: &mut BTreeMap<String, ResourceState>,
    edges: &mut BTreeMap<(String, String, String), LinkEdge>,
) {
    for (head, batch) in inputs {
        for rref in &batch.resources {
            let Some(resolved) =
                resolve_resource(workspace, batch, &rref.resource_type, &rref.raw_target)
            else {
                continue;
            };
            let id = resource_id(&resolved.resource_type, &resolved.identity);
            let entry = resources
                .entry(id.to_string())
                .or_insert_with(|| ResourceState {
                    id: id.clone(),
                    resource_type: resolved.resource_type.clone(),
                    identity: resolved.identity.clone(),
                    mechanism: rref.mechanism.clone(),
                    commits: Vec::new(),
                });
            if entry.mechanism.is_empty() && !rref.mechanism.is_empty() {
                entry.mechanism = rref.mechanism.clone();
            }
            let commit = head.commit_id.to_string();
            if !entry.commits.contains(&commit) {
                entry.commits.push(commit);
                entry.commits.sort();
            }

            if rref.node_id.as_str().is_empty() {
                continue;
            }
            let Some(kind) = resource_edge_kind(&rref.access) else {
                continue;
            };
            let mut props = Map::new();
            if !rref.mechanism.is_empty() {
                props.insert("mechanism".to_string(), json!(rref.mechanism));
            }
            props.insert("confidence".to_string(), json!(rank_str(resolved.rank)));
            set_evidence(&mut props, resolved.evidence);
            props.insert("resource_type".to_string(), json!(resolved.resource_type));
            props.insert("access".to_string(), json!(rref.access));
            upsert_link(
                edges,
                LinkEdge {
                    kind,
                    from: rref.node_id.clone(),
                    to: id,
                    properties: props,
                    branch_pairs: vec![head.label()],
                },
            );
        }
    }
}

/// Compute the workspace link set from per-branch heads and their persisted
/// contract batches.
pub fn compute_links(
    workspace: &WorkspaceConfig,
    inputs: &[(BranchHead, ContractBatch)],
) -> LinkSet {
    let mut sorted: Vec<(BranchHead, ContractBatch)> = inputs.to_vec();
    sorted.sort_by(|a, b| {
        (a.0.repo_id.as_str(), a.0.branch.as_str())
            .cmp(&(b.0.repo_id.as_str(), b.0.branch.as_str()))
    });

    let mut edges: BTreeMap<(String, String, String), LinkEdge> = BTreeMap::new();
    let mut topics: BTreeMap<String, TopicState> = BTreeMap::new();
    let mut resources: BTreeMap<String, ResourceState> = BTreeMap::new();

    match_service_links(workspace, &sorted, &mut resources, &mut edges);
    match_channel_links(workspace, &sorted, &mut topics, &mut edges);
    match_resource_links(workspace, &sorted, &mut resources, &mut edges);

    let mut set = LinkSet {
        edges: edges.into_values().collect(),
        topics: topics.into_values().collect(),
        resources: resources.into_values().collect(),
    };
    set.sort();
    set
}

fn contains_pairs(set: &LinkSet) -> BTreeSet<(String, String)> {
    set.topics
        .iter()
        .flat_map(|topic| {
            topic
                .commits
                .iter()
                .map(|commit| (commit.clone(), topic.id.to_string()))
        })
        .chain(set.resources.iter().flat_map(|resource| {
            resource
                .commits
                .iter()
                .map(|commit| (commit.clone(), resource.id.to_string()))
        }))
        .collect()
}

/// Compute the delta that turns `old` into `new`.
///
/// Emission order (locked): topic creates, topic updates, resource creates,
/// resource updates, service/resource edge upserts, edge deletes,
/// `ContainsStateOf` creates, stale `ContainsStateOf` deletes, orphaned
/// topic deletes, orphaned resource deletes (last — the store's guarded
/// delete requires zero incoming `ContainsStateOf`).
pub fn diff_link_sets(old: &LinkSet, new: &LinkSet) -> GraphDelta {
    let mut ops = Vec::new();

    let old_topics: BTreeMap<&str, &TopicState> = old
        .topics
        .iter()
        .map(|topic| (topic.id.as_str(), topic))
        .collect();
    let new_topics: BTreeMap<&str, &TopicState> = new
        .topics
        .iter()
        .map(|topic| (topic.id.as_str(), topic))
        .collect();

    for (id, topic) in &new_topics {
        if !old_topics.contains_key(id) {
            ops.push(DeltaOp::CreateNode(topic.to_graph_node()));
        }
    }
    for (id, topic) in &new_topics {
        if let Some(prev) = old_topics.get(id)
            && prev.content_differs(topic)
        {
            ops.push(DeltaOp::UpdateNode(topic.to_graph_node()));
        }
    }

    let old_resources: BTreeMap<&str, &ResourceState> = old
        .resources
        .iter()
        .map(|resource| (resource.id.as_str(), resource))
        .collect();
    let new_resources: BTreeMap<&str, &ResourceState> = new
        .resources
        .iter()
        .map(|resource| (resource.id.as_str(), resource))
        .collect();

    for (id, resource) in &new_resources {
        if !old_resources.contains_key(id) {
            ops.push(DeltaOp::CreateNode(resource.to_graph_node()));
        }
    }
    for (id, resource) in &new_resources {
        if let Some(prev) = old_resources.get(id)
            && prev.content_differs(resource)
        {
            ops.push(DeltaOp::UpdateNode(resource.to_graph_node()));
        }
    }

    let old_edges: BTreeMap<(String, String, String), &LinkEdge> =
        old.edges.iter().map(|edge| (edge.key(), edge)).collect();
    let new_edges: BTreeMap<(String, String, String), &LinkEdge> =
        new.edges.iter().map(|edge| (edge.key(), edge)).collect();

    for (key, edge) in &new_edges {
        match old_edges.get(key) {
            None => ops.push(DeltaOp::CreateRelationship(edge.to_graph_edge())),
            Some(prev) => {
                let prev_graph = prev.to_graph_edge();
                let new_graph = edge.to_graph_edge();
                if prev_graph.content_hash != new_graph.content_hash
                    || prev_graph.properties != new_graph.properties
                {
                    ops.push(DeltaOp::UpdateRelationship(new_graph));
                }
            }
        }
    }
    for (key, prev) in &old_edges {
        if !new_edges.contains_key(key) {
            ops.push(DeltaOp::DeleteRelationship {
                kind: prev.kind,
                from: prev.from.clone(),
                to: prev.to.clone(),
            });
        }
    }

    let old_contains = contains_pairs(old);
    let new_contains = contains_pairs(new);
    for (commit, topic) in new_contains.difference(&old_contains) {
        ops.push(DeltaOp::CreateRelationship(
            GraphEdge::new(
                RelationKind::ContainsStateOf,
                CanonicalId::from(commit.clone()),
                CanonicalId::from(topic.clone()),
            )
            .with_provenance(link_provenance()),
        ));
    }
    for (commit, topic) in old_contains.difference(&new_contains) {
        ops.push(DeltaOp::DeleteRelationship {
            kind: RelationKind::ContainsStateOf,
            from: CanonicalId::from(commit.clone()),
            to: CanonicalId::from(topic.clone()),
        });
    }

    for id in old_topics.keys() {
        if !new_topics.contains_key(id) {
            ops.push(DeltaOp::DeleteNode(CanonicalId::new(id.to_string())));
        }
    }
    for id in old_resources.keys() {
        if !new_resources.contains_key(id) {
            ops.push(DeltaOp::DeleteNode(CanonicalId::new(id.to_string())));
        }
    }

    GraphDelta {
        ops,
        base_revision: None,
        target_revision: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{ChannelRef, EvidenceRecord};

    fn head(repo: &str, branch: &str) -> BranchHead {
        BranchHead {
            repo_id: repo.to_string(),
            github: format!("org/{repo}"),
            branch: branch.to_string(),
            sha: "sha1".to_string(),
            commit_id: CanonicalId::from(format!("{repo}@{branch}")),
        }
    }

    fn provide(route: &str, handler: &str) -> ProvidedEndpoint {
        ProvidedEndpoint {
            file: "server.rs".to_string(),
            mechanism: Mechanism::Rest,
            http_method: "GET".to_string(),
            route: route.to_string(),
            service: String::new(),
            method: String::new(),
            handler_id: CanonicalId::from(handler),
            framework: "axum".to_string(),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    fn consume(route: &str, caller: &str) -> ConsumedContract {
        ConsumedContract {
            file: "client.rs".to_string(),
            mechanism: Mechanism::Rest,
            http_method: "GET".to_string(),
            route: route.to_string(),
            service: String::new(),
            method: String::new(),
            caller_id: CanonicalId::from(caller),
            target_hint: String::new(),
            framework: String::new(),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    fn provider(repo: &str, branch: &str, routes: &[(&str, &str)]) -> (BranchHead, ContractBatch) {
        let mut batch = ContractBatch::default();
        for (route, handler) in routes {
            batch.provides.push(provide(route, handler));
        }
        (head(repo, branch), batch)
    }

    fn consumer(
        repo: &str,
        branch: &str,
        consumes: Vec<ConsumedContract>,
    ) -> (BranchHead, ContractBatch) {
        let batch = ContractBatch {
            consumes,
            ..ContractBatch::default()
        };
        (head(repo, branch), batch)
    }

    fn workspace() -> WorkspaceConfig {
        WorkspaceConfig::default()
    }

    #[test]
    fn rest_single_unrestricted_candidate_is_medium() {
        let ws = workspace();
        let inputs = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/orders/{id}", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/orders/{id}", "orders.handler")]),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 1);
        let edge = &set.edges[0];
        assert_eq!(edge.kind, RelationKind::CallsApi);
        assert_eq!(edge.from, CanonicalId::from("billing.caller"));
        assert_eq!(edge.to, CanonicalId::from("orders.handler"));
        assert_eq!(edge.properties["confidence"], json!("medium"));
        assert_eq!(edge.properties["mechanism"], json!("rest"));
        assert_eq!(edge.properties["route"], json!("/orders/*"));
        assert_eq!(
            edge.properties["evidence"],
            json!([{"kind": "route", "detail": "/orders/*"}])
        );
        assert_eq!(
            edge.branch_pairs,
            vec!["billing@main -> orders-api@main".to_string()]
        );
        assert!(set.topics.is_empty());
    }

    #[test]
    fn base_url_restriction_promotes_to_high_with_target_hint_evidence() {
        let mut ws = workspace();
        ws.links.base_urls.insert(
            "https://api.orders.internal".to_string(),
            "orders-api".to_string(),
        );

        let mut c = consume("/orders", "billing.caller");
        c.target_hint = "https://api.orders.internal/v1/orders".to_string();
        let inputs = vec![
            consumer("billing", "main", vec![c]),
            provider("orders-api", "main", &[("/orders", "orders.handler")]),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 1);
        assert_eq!(set.edges[0].properties["confidence"], json!("high"));
        let evidence = evidence_of(&set.edges[0].properties);
        assert!(evidence.contains(&(
            "target_hint".to_string(),
            "https://api.orders.internal/v1/orders".to_string()
        )));
    }

    #[test]
    fn base_url_prefix_requires_boundary() {
        let mut base_urls = BTreeMap::new();
        base_urls.insert(
            "https://api.orders.internal".to_string(),
            "orders-api".to_string(),
        );
        assert_eq!(
            base_url_repo(&base_urls, "https://api.orders.internal/orders"),
            Some("orders-api".to_string())
        );
        assert_eq!(
            base_url_repo(&base_urls, "https://api.orders.internal"),
            Some("orders-api".to_string())
        );
        assert_eq!(
            base_url_repo(&base_urls, "https://api.orders.internal.evil.test/x"),
            None,
            "host suffix must not match"
        );
        assert_eq!(base_url_repo(&base_urls, ""), None);
    }

    #[test]
    fn evidence_restriction_matches_openapi_records() {
        let ws = workspace();
        let mut cbatch = ContractBatch::default();
        cbatch
            .consumes
            .push(consume("/orders/{id}", "billing.caller"));
        cbatch.evidence.push(EvidenceRecord {
            file: "openapi.yaml".to_string(),
            kind: EvidenceKind::Openapi,
            value: "GET /orders/{id}".to_string(),
            detail: "get:".to_string(),
            target_repo: "orders-api".to_string(),
            channel: String::new(),
            broker: String::new(),
            line: 5,
        });
        let inputs = vec![
            (head("billing", "main"), cbatch),
            provider("orders-api", "main", &[("/orders/{id}", "orders.handler")]),
            provider(
                "payments-api",
                "main",
                &[("/orders/{id}", "payments.handler")],
            ),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 1, "restriction drops the other provider");
        assert_eq!(set.edges[0].to, CanonicalId::from("orders.handler"));
        assert_eq!(set.edges[0].properties["confidence"], json!("high"));
        assert!(
            evidence_of(&set.edges[0].properties)
                .contains(&("openapi".to_string(), "GET /orders/{id}".to_string()))
        );
    }

    #[test]
    fn multiple_candidate_repos_link_all_at_low_with_candidates_evidence() {
        let ws = workspace();
        let inputs = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/orders", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/orders", "orders.handler")]),
            provider("billing-api", "main", &[("/orders", "billing.handler")]),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 2, "both candidates linked");
        assert!(
            set.edges
                .iter()
                .all(|e| e.properties["confidence"] == json!("low"))
        );
        let evidence = evidence_of(&set.edges[0].properties);
        assert!(evidence.contains(&(
            "candidates".to_string(),
            "billing-api,orders-api".to_string()
        )));
    }

    #[test]
    fn generic_routes_are_excluded() {
        let ws = workspace();
        let inputs = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/health", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/health", "orders.handler")]),
        ];
        assert!(compute_links(&ws, &inputs).is_empty());

        let inputs = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/metrics", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/api/orders", "orders.handler")]),
        ];
        assert!(compute_links(&ws, &inputs).is_empty());
    }

    #[test]
    fn grpc_service_boundary_and_confidence_rules() {
        let ws = workspace();
        let mut c = consume("/ignored", "billing.caller");
        c.mechanism = Mechanism::Grpc;
        c.service = "v1.OrderService".to_string();
        c.method = "GetOrder".to_string();

        let mut p = provide("/ignored", "orders.handler");
        p.mechanism = Mechanism::Grpc;
        p.service = "orders.v1.OrderService".to_string();
        p.method = "GetOrder".to_string();

        let inputs = vec![
            consumer("billing", "main", vec![c.clone()]),
            provider("orders-api", "main", &[("/ignored", "orders.handler")]),
        ];
        let mut set = compute_links(&ws, &inputs);
        assert!(set.edges.is_empty(), "provider route/method mismatch");

        let inputs = vec![
            (head("billing", "main"), {
                let mut b = ContractBatch::default();
                b.consumes.push(c.clone());
                b
            }),
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.provides.push(p.clone());
                b
            }),
        ];
        set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 1, "boundary service match links");
        assert_eq!(set.edges[0].properties["confidence"], json!("medium"));
        assert_eq!(set.edges[0].properties["service"], json!("v1.OrderService"));
        assert_eq!(set.edges[0].properties["method"], json!("GetOrder"));

        p.method = "OtherOrder".to_string();
        let inputs = vec![
            (head("billing", "main"), {
                let mut b = ContractBatch::default();
                b.consumes.push(c.clone());
                b
            }),
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.provides.push(p.clone());
                b
            }),
        ];
        assert!(compute_links(&ws, &inputs).is_empty(), "method mismatch");

        p.method = "GetOrder".to_string();
        c.service = String::new();
        let inputs = vec![
            (head("billing", "main"), {
                let mut b = ContractBatch::default();
                b.consumes.push(c);
                b
            }),
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.provides.push(p);
                b
            }),
        ];
        set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 1);
        assert_eq!(
            set.edges[0].properties["confidence"],
            json!("low"),
            "empty consumer service drops to low"
        );
    }

    #[test]
    fn jsonrpc_requires_equal_method_and_route_when_both_set() {
        let ws = workspace();
        let mut c = consume("", "billing.caller");
        c.mechanism = Mechanism::JsonRpc;
        c.method = "orders.get".to_string();
        c.route = "/rpc".to_string();

        let mut p = provide("", "orders.handler");
        p.mechanism = Mechanism::JsonRpc;
        p.method = "orders.get".to_string();
        p.route = "/rpc".to_string();

        let inputs = vec![
            (head("billing", "main"), {
                let mut b = ContractBatch::default();
                b.consumes.push(c.clone());
                b
            }),
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.provides.push(p.clone());
                b
            }),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.edges.len(), 1);
        assert_eq!(set.edges[0].properties["route"], json!("/rpc"));

        p.route = "/other".to_string();
        let inputs = vec![
            (head("billing", "main"), {
                let mut b = ContractBatch::default();
                b.consumes.push(c);
                b
            }),
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.provides.push(p);
                b
            }),
        ];
        assert!(compute_links(&ws, &inputs).is_empty(), "route mismatch");
    }

    #[test]
    fn messaging_creates_shared_topic_with_edges_and_commits() {
        let ws = workspace();
        let mut publish = ChannelRef {
            file: "src/publish.rs".to_string(),
            direction: ChannelDirection::Publish,
            broker: "kafka".to_string(),
            channel_type: String::new(),
            channel: "Orders.Created".to_string(),
            routing_key: String::new(),
            node_id: CanonicalId::from("orders.publisher"),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        };
        publish.channel = "orders.created".to_string();
        let subscribe = ChannelRef {
            file: "src/subscribe.rs".to_string(),
            direction: ChannelDirection::Subscribe,
            broker: "KAFKA".to_string(),
            channel_type: String::new(),
            channel: "orders.created".to_string(),
            routing_key: String::new(),
            node_id: CanonicalId::from("billing.subscriber"),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        };

        let inputs = vec![
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.channels.push(publish);
                b
            }),
            (head("billing", "main"), {
                let mut b = ContractBatch::default();
                b.channels.push(subscribe);
                b
            }),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.topics.len(), 1, "shared topic hub");
        let topic = &set.topics[0];
        assert_eq!(topic.broker, "kafka");
        assert_eq!(topic.channel, "orders.created");
        assert_eq!(topic.channel_type, "topic");
        assert_eq!(topic.commits, vec!["billing@main", "orders-api@main"]);

        assert_eq!(set.edges.len(), 2);
        let publish_edge = set
            .edges
            .iter()
            .find(|e| e.kind == RelationKind::PublishTo)
            .expect("publish edge");
        assert_eq!(publish_edge.from, CanonicalId::from("orders.publisher"));
        assert_eq!(publish_edge.to, topic.id);
        assert_eq!(publish_edge.properties["confidence"], json!("medium"));

        let consume_edge = set
            .edges
            .iter()
            .find(|e| e.kind == RelationKind::ConsumeFrom)
            .expect("consume edge");
        assert_eq!(consume_edge.from, CanonicalId::from("billing.subscriber"));
    }

    #[test]
    fn messaging_config_broker_promotes_to_high_and_infers_queue() {
        let mut ws = workspace();
        ws.links
            .channel_brokers
            .insert("payments.settled".to_string(), "rabbitmq".to_string());

        let channel = ChannelRef {
            file: "src/pay.rs".to_string(),
            direction: ChannelDirection::Publish,
            broker: String::new(),
            channel_type: String::new(),
            channel: "payments.settled".to_string(),
            routing_key: String::new(),
            node_id: CanonicalId::from("pay.publisher"),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        };
        let inputs = vec![(head("payments", "main"), {
            let mut b = ContractBatch::default();
            b.channels.push(channel);
            b
        })];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.topics.len(), 1);
        assert_eq!(set.topics[0].broker, "rabbitmq");
        assert_eq!(set.topics[0].channel_type, "queue");
        assert_eq!(set.edges.len(), 1);
        assert_eq!(set.edges[0].properties["confidence"], json!("high"));
        assert_eq!(set.edges[0].properties["mechanism"], json!("rabbitmq"));
    }

    #[test]
    fn messaging_skips_empty_and_unresolved_template_channels() {
        let ws = workspace();
        let make = |channel: &str| ChannelRef {
            file: "src/x.rs".to_string(),
            direction: ChannelDirection::Publish,
            broker: "kafka".to_string(),
            channel_type: String::new(),
            channel: channel.to_string(),
            routing_key: String::new(),
            node_id: CanonicalId::from("n1"),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        };
        let inputs = vec![(head("orders", "main"), {
            let mut b = ContractBatch::default();
            b.channels.push(make(""));
            b.channels.push(make("${ENV}_topic"));
            b.channels.push(make("orders.ready"));
            b
        })];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.topics.len(), 1);
        assert_eq!(set.topics[0].channel, "orders.ready");
    }

    #[test]
    fn diff_empty_to_populated_then_idempotent_then_teardown() {
        let ws = workspace();
        let inputs = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/orders", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/orders", "orders.handler")]),
            (head("orders-api", "main"), {
                let mut b = ContractBatch::default();
                b.channels.push(ChannelRef {
                    file: "src/publish.rs".to_string(),
                    direction: ChannelDirection::Publish,
                    broker: "kafka".to_string(),
                    channel_type: String::new(),
                    channel: "orders.created".to_string(),
                    routing_key: String::new(),
                    node_id: CanonicalId::from("orders.publisher"),
                    language: "rust".to_string(),
                    location: None,
                    properties: Default::default(),
                });
                b
            }),
        ];
        let new_set = compute_links(&ws, &inputs);

        let create = diff_link_sets(&LinkSet::default(), &new_set);
        assert!(create.ops.iter().any(
            |op| matches!(op, DeltaOp::CreateNode(n) if n.kind == ckg_domain::NodeKind::Topic)
        ));
        assert!(create.ops.iter().any(
            |op| matches!(op, DeltaOp::CreateRelationship(e) if e.kind == RelationKind::CallsApi)
        ));
        assert!(create.ops.iter().any(
            |op| matches!(op, DeltaOp::CreateRelationship(e) if e.kind == RelationKind::PublishTo)
        ));
        assert!(create.ops.iter().any(|op| matches!(op, DeltaOp::CreateRelationship(e) if e.kind == RelationKind::ContainsStateOf)));
        let topic_idx = create
            .ops
            .iter()
            .position(|op| matches!(op, DeltaOp::CreateNode(_)))
            .unwrap();
        let contains_idx = create.ops.iter().position(|op| matches!(op, DeltaOp::CreateRelationship(e) if e.kind == RelationKind::ContainsStateOf)).unwrap();
        assert!(topic_idx < contains_idx, "topic created before containment");

        let idempotent = diff_link_sets(&new_set, &new_set);
        assert!(idempotent.is_empty(), "diff(set, set) must be empty");

        let teardown = diff_link_sets(&new_set, &LinkSet::default());
        assert!(teardown.ops.iter().any(|op| matches!(
            op,
            DeltaOp::DeleteRelationship {
                kind: RelationKind::CallsApi,
                ..
            }
        )));
        assert!(teardown.ops.iter().any(|op| matches!(
            op,
            DeltaOp::DeleteRelationship {
                kind: RelationKind::PublishTo,
                ..
            }
        )));
        assert!(teardown.ops.iter().any(|op| matches!(
            op,
            DeltaOp::DeleteRelationship {
                kind: RelationKind::ContainsStateOf,
                ..
            }
        )));
        let delete_idx = teardown
            .ops
            .iter()
            .position(|op| matches!(op, DeltaOp::DeleteNode(_)))
            .unwrap();
        let contains_delete_idx = teardown
            .ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    DeltaOp::DeleteRelationship {
                        kind: RelationKind::ContainsStateOf,
                        ..
                    }
                )
            })
            .unwrap();
        assert!(
            contains_delete_idx < delete_idx,
            "topic delete must come last so the guarded delete succeeds"
        );
    }

    #[test]
    fn diff_updates_edge_when_branch_pairs_grow() {
        let ws = workspace();
        let inputs_a = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/orders", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/orders", "orders.handler")]),
        ];
        let set_a = compute_links(&ws, &inputs_a);

        let inputs_b = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/orders", "billing.caller")],
            ),
            consumer(
                "billing",
                "develop",
                vec![consume("/orders", "billing.caller")],
            ),
            provider("orders-api", "main", &[("/orders", "orders.handler")]),
        ];
        let set_b = compute_links(&ws, &inputs_b);

        let delta = diff_link_sets(&set_a, &set_b);
        assert!(
            delta
                .ops
                .iter()
                .any(|op| matches!(op, DeltaOp::UpdateRelationship(_))),
            "branch_pairs union must update the edge: {:?}",
            delta.ops
        );
        assert_eq!(set_b.edges[0].branch_pairs.len(), 2);
    }

    #[test]
    fn compute_is_deterministic_regardless_of_input_order() {
        let ws = workspace();
        let a = consumer(
            "billing",
            "main",
            vec![consume("/orders", "billing.caller")],
        );
        let b = provider("orders-api", "main", &[("/orders", "orders.handler")]);
        let forward = compute_links(&ws, &[a.clone(), b.clone()]);
        let reverse = compute_links(&ws, &[b, a]);
        assert_eq!(forward, reverse);
    }

    fn resource(
        node: &str,
        resource_type: &str,
        mechanism: &str,
        access: &str,
        raw_target: &str,
    ) -> ckg_domain::ResourceRef {
        ckg_domain::ResourceRef {
            file: "src/lib.rs".to_string(),
            resource_type: resource_type.to_string(),
            mechanism: mechanism.to_string(),
            access: access.to_string(),
            raw_target: raw_target.to_string(),
            node_id: CanonicalId::from(node),
            language: "python".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    fn with_resources(
        repo: &str,
        branch: &str,
        resources: Vec<ckg_domain::ResourceRef>,
    ) -> (BranchHead, ContractBatch) {
        (
            head(repo, branch),
            ContractBatch {
                resources,
                ..ContractBatch::default()
            },
        )
    }

    #[test]
    fn resource_refs_emit_fine_grained_edges_and_shared_nodes() {
        let ws = workspace();
        let inputs = vec![
            with_resources(
                "billing",
                "main",
                vec![
                    resource(
                        "billing.read",
                        "bucket",
                        "s3",
                        "read",
                        "s3://orders-exports/2024/a.json",
                    ),
                    resource("billing.write", "bucket", "s3", "write", "orders-exports"),
                ],
            ),
            with_resources(
                "reporting",
                "main",
                vec![resource(
                    "reporting.read",
                    "bucket",
                    "s3",
                    "read",
                    "https://storage.googleapis.com/orders-exports/x",
                )],
            ),
        ];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.resources.len(), 1, "one shared bucket node");
        let node = &set.resources[0];
        assert_eq!(node.resource_type, "bucket");
        assert_eq!(node.identity, "orders-exports");
        assert_eq!(node.mechanism, "s3");
        assert_eq!(
            node.commits,
            vec!["billing@main".to_string(), "reporting@main".to_string()]
        );
        assert_eq!(node.to_graph_node().kind, ckg_domain::NodeKind::Resource);

        assert_eq!(set.edges.len(), 3);
        let reads: Vec<_> = set
            .edges
            .iter()
            .filter(|e| e.kind == RelationKind::ReadsFrom)
            .collect();
        assert_eq!(reads.len(), 2, "two readers across repos");
        for edge in &reads {
            assert_eq!(edge.to, node.id);
            assert_eq!(edge.properties["resource_type"], json!("bucket"));
            assert_eq!(edge.properties["access"], json!("read"));
        }
        let write = set
            .edges
            .iter()
            .find(|e| e.kind == RelationKind::WritesTo)
            .expect("write edge");
        assert_eq!(write.from, CanonicalId::from("billing.write"));
        assert_eq!(write.properties["confidence"], json!("medium"));
        let read_high = reads
            .iter()
            .find(|e| e.from == CanonicalId::from("billing.read"))
            .unwrap();
        assert_eq!(
            read_high.properties["confidence"],
            json!("high"),
            "literal URL target ranks high"
        );
        assert_eq!(
            read_high.properties["evidence"],
            json!([{"kind": "resource", "detail": "orders-exports"}])
        );
    }

    #[test]
    fn resource_registry_and_unresolved_env_alias() {
        let mut ws = workspace();
        ws.links.resources.insert(
            "env:ORDERS_DSN".to_string(),
            "database:orders-db".to_string(),
        );
        let inputs = vec![with_resources(
            "billing",
            "main",
            vec![
                resource(
                    "billing.repo",
                    "database",
                    "postgres",
                    "connect",
                    "env:ORDERS_DSN",
                ),
                resource("billing.repo", "database", "", "connect", "env:UNKNOWN_DSN"),
            ],
        )];
        let set = compute_links(&ws, &inputs);
        let resolved = set
            .resources
            .iter()
            .find(|r| r.identity == "orders-db")
            .expect("registry-resolved node");
        assert_eq!(resolved.resource_type, "database");
        let edge = set
            .edges
            .iter()
            .find(|e| e.to == resolved.id)
            .expect("edge to resolved node");
        assert_eq!(edge.properties["confidence"], json!("high"));
        assert_eq!(
            edge.properties["evidence"],
            json!([{"kind": "registry", "detail": "env:ORDERS_DSN"}])
        );

        let unresolved = set
            .resources
            .iter()
            .find(|r| r.identity == "env:UNKNOWN_DSN")
            .expect("low-confidence placeholder");
        let edge = set
            .edges
            .iter()
            .find(|e| e.to == unresolved.id)
            .expect("edge to placeholder");
        assert_eq!(edge.properties["confidence"], json!("low"));
        assert_eq!(
            edge.properties["evidence"],
            json!([{"kind": "env", "detail": "UNKNOWN_DSN"}])
        );
    }

    #[test]
    fn unmatched_rest_consume_links_to_external_api_resource() {
        let ws = workspace();
        let mut c = consume("/v1/widgets", "billing.caller");
        c.target_hint = "https://widgets.vendor.example/v1/widgets".to_string();
        let inputs = vec![consumer("billing", "main", vec![c])];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.resources.len(), 1);
        let api = &set.resources[0];
        assert_eq!(api.resource_type, "api");
        assert_eq!(api.identity, "widgets.vendor.example");
        assert_eq!(api.commits, vec!["billing@main".to_string()]);
        assert_eq!(set.edges.len(), 1);
        let edge = &set.edges[0];
        assert_eq!(edge.kind, RelationKind::CallsApi);
        assert_eq!(edge.from, CanonicalId::from("billing.caller"));
        assert_eq!(edge.to, api.id);
        assert_eq!(edge.properties["confidence"], json!("low"));
        assert_eq!(edge.properties["resource_type"], json!("api"));
        assert_eq!(
            edge.properties["evidence"],
            json!([{"kind": "unmatched", "detail": "widgets.vendor.example"}])
        );
    }

    #[test]
    fn unmatched_rest_without_hint_falls_back_to_route_identity() {
        let ws = workspace();
        let inputs = vec![consumer(
            "billing",
            "main",
            vec![consume("/v1/widgets", "billing.caller")],
        )];
        let set = compute_links(&ws, &inputs);
        assert_eq!(set.resources.len(), 1);
        assert_eq!(set.resources[0].identity, "/v1/widgets");
        assert_eq!(set.edges.len(), 1);
        assert_eq!(set.edges[0].kind, RelationKind::CallsApi);
    }

    #[test]
    fn unmatched_generic_routes_and_non_http_mechanisms_skip_fallback() {
        let ws = workspace();
        let inputs = vec![consumer(
            "billing",
            "main",
            vec![consume("/health", "billing.caller")],
        )];
        let set = compute_links(&ws, &inputs);
        assert!(set.is_empty());

        let mut c = consume("/ignored", "billing.caller");
        c.mechanism = Mechanism::Grpc;
        c.service = "v1.Widgets".to_string();
        c.method = "Get".to_string();
        let inputs = vec![consumer("billing", "main", vec![c])];
        let set = compute_links(&ws, &inputs);
        assert!(set.is_empty(), "grpc unmatched stays dropped: {:?}", set);
    }

    #[test]
    fn matched_provides_win_over_api_fallback() {
        let ws = workspace();
        let inputs = vec![
            consumer(
                "billing",
                "main",
                vec![consume("/widgets", "billing.caller")],
            ),
            provider("widgets-api", "main", &[("/widgets", "widgets.handler")]),
        ];
        let set = compute_links(&ws, &inputs);
        assert!(
            set.resources.is_empty(),
            "matched consume must not mint an api node"
        );
        assert_eq!(set.edges.len(), 1);
        assert_eq!(set.edges[0].to, CanonicalId::from("widgets.handler"));
    }

    #[test]
    fn resource_node_lifecycle_diff_and_gc() {
        let ws = workspace();
        let inputs = vec![with_resources(
            "billing",
            "main",
            vec![resource(
                "billing.read",
                "bucket",
                "s3",
                "read",
                "s3://orders-exports/a",
            )],
        )];
        let full = compute_links(&ws, &inputs);
        assert_eq!(full.resources.len(), 1);
        assert_eq!(full.edges.len(), 1);

        let create = diff_link_sets(&LinkSet::default(), &full);
        let node_idx = create
            .ops
            .iter()
            .position(|op| matches!(op, DeltaOp::CreateNode(n) if n.kind == ckg_domain::NodeKind::Resource))
            .expect("resource create");
        let edge_idx = create.ops.iter().position(|op| matches!(op, DeltaOp::CreateRelationship(e) if e.kind == RelationKind::ReadsFrom)).expect("edge create");
        let contains_idx = create.ops.iter().position(|op| matches!(op, DeltaOp::CreateRelationship(e) if e.kind == RelationKind::ContainsStateOf)).expect("contains create");
        assert!(
            node_idx < edge_idx && edge_idx < contains_idx,
            "node before edge before contains: {node_idx} {edge_idx} {contains_idx}"
        );
        assert!(
            diff_link_sets(&full, &full).is_empty(),
            "diff(set, set) must be empty"
        );

        let teardown = diff_link_sets(&full, &LinkSet::default());
        let contains_delete_idx = teardown
            .ops
            .iter()
            .position(|op| {
                matches!(
                    op,
                    DeltaOp::DeleteRelationship {
                        kind: RelationKind::ContainsStateOf,
                        ..
                    }
                )
            })
            .expect("contains delete");
        let node_delete_idx = teardown
            .ops
            .iter()
            .position(|op| matches!(op, DeltaOp::DeleteNode(_)))
            .expect("node delete");
        assert!(
            contains_delete_idx < node_delete_idx,
            "resource delete must come after stale contains: {contains_delete_idx} < {node_delete_idx}"
        );
    }
}
