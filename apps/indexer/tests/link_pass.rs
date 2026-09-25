use ckg_domain::{
    CanonicalId, ChannelDirection, ChannelRef, ConsumedContract, ContractBatch, Mechanism,
    ProvidedEndpoint, RelationKind,
};
use ckg_graph_api::MockStore;
use ckg_indexer::jobs::run_link_pass;
use ckg_indexer::state::IndexState;
use ckg_links::{contracts_path, link_set_path, load_link_set, save_contracts};
use ckg_repository_config::{RepositoryConfig, WorkspaceConfig};

fn provide(file: &str, route: &str, handler: &str) -> ProvidedEndpoint {
    ProvidedEndpoint {
        file: file.to_string(),
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

fn consume(file: &str, route: &str, caller: &str) -> ConsumedContract {
    ConsumedContract {
        file: file.to_string(),
        mechanism: Mechanism::Rest,
        http_method: "GET".to_string(),
        route: route.to_string(),
        service: String::new(),
        method: String::new(),
        caller_id: CanonicalId::from(caller),
        target_hint: String::new(),
        framework: "reqwest".to_string(),
        language: "rust".to_string(),
        location: None,
        properties: Default::default(),
    }
}

fn channel(file: &str, direction: ChannelDirection, node: &str) -> ChannelRef {
    ChannelRef {
        file: file.to_string(),
        direction,
        broker: "kafka".to_string(),
        channel_type: String::new(),
        channel: "orders.created".to_string(),
        routing_key: String::new(),
        node_id: CanonicalId::from(node),
        language: "rust".to_string(),
        location: None,
        properties: Default::default(),
    }
}

fn workspace() -> WorkspaceConfig {
    let mut workspace = WorkspaceConfig::default();
    workspace.links.enabled = true;
    for id in ["prov", "cons", "ghost"] {
        workspace.repositories.push(RepositoryConfig::new(
            id,
            format!("acme/{id}"),
            format!("https://example.com/{id}.git"),
        ));
    }
    workspace
}

fn indexed_state() -> IndexState {
    let mut state = IndexState::default();
    state.record("prov", "main", "sha-prov".to_string(), "unix:1".to_string());
    state.record("cons", "main", "sha-cons".to_string(), "unix:1".to_string());
    state
}

fn seed_contracts(cache: &std::path::Path) {
    let mut provider = ContractBatch::default();
    provider
        .provides
        .push(provide("server.rs", "/orders/{id}", "prov.handler"));
    provider.channels.push(channel(
        "subscribe.rs",
        ChannelDirection::Subscribe,
        "prov.sub",
    ));
    save_contracts(&contracts_path(cache, "prov", "main"), &provider)
        .expect("save provider contracts");

    let mut consumer = ContractBatch::default();
    consumer
        .consumes
        .push(consume("client.rs", "/orders/{id}", "cons.caller"));
    consumer
        .channels
        .push(channel("publish.rs", ChannelDirection::Publish, "cons.pub"));
    save_contracts(&contracts_path(cache, "cons", "main"), &consumer)
        .expect("save consumer contracts");

    let mut ghost = ContractBatch::default();
    ghost
        .provides
        .push(provide("ghost.rs", "/orders/{id}", "ghost.handler"));
    save_contracts(&contracts_path(cache, "ghost", "main"), &ghost).expect("save ghost contracts");
}

#[tokio::test]
async fn link_pass_builds_then_prunes_the_link_set() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let workspace = workspace();
    let state = indexed_state();
    seed_contracts(cache_dir.path());

    let store = MockStore::new();
    let report = run_link_pass(&store, &workspace, cache_dir.path(), &state)
        .await
        .expect("link pass");
    assert_eq!(report.edges, 3, "calls + publish + consume");
    assert_eq!(report.topics, 1, "one shared topic hub");
    assert_eq!(report.ops_applied, 6, "topic, three links, two contains");
    assert!(
        link_set_path(cache_dir.path()).exists(),
        "links.json must be written"
    );
    assert_eq!(store.node_count(), 1, "topic node applied");
    assert_eq!(store.edge_count(), 5, "three links plus two contains");

    let set = load_link_set(&link_set_path(cache_dir.path())).expect("load link set");
    assert_eq!(set.edges.len(), 3);
    assert_eq!(set.topics.len(), 1);
    assert_eq!(set.topics[0].channel, "orders.created");
    assert!(
        set.edges
            .iter()
            .all(|edge| edge.to != CanonicalId::from("ghost.handler")),
        "branches without index state are skipped"
    );
    let calls = set
        .edges
        .iter()
        .find(|edge| edge.kind == RelationKind::CallsApi)
        .expect("calls edge");
    assert_eq!(calls.from, CanonicalId::from("cons.caller"));
    assert_eq!(calls.to, CanonicalId::from("prov.handler"));

    let unchanged = run_link_pass(&store, &workspace, cache_dir.path(), &state)
        .await
        .expect("second link pass");
    assert_eq!(unchanged.ops_applied, 0, "unchanged link set is a no-op");
    assert_eq!(store.edge_count(), 5, "store untouched when unchanged");

    save_contracts(
        &contracts_path(cache_dir.path(), "prov", "main"),
        &ContractBatch::default(),
    )
    .expect("shrink provider contracts");
    let pruned = run_link_pass(&store, &workspace, cache_dir.path(), &state)
        .await
        .expect("pruning link pass");
    // The consumer's `/orders/{id}` consume no longer matches a provider, so
    // it falls back to an external `CallsApi -> Resource(api)` edge.
    assert_eq!(pruned.edges, 2, "publish plus the unmatched api fallback");
    assert_eq!(pruned.topics, 1, "topic survives via the consumer");
    assert_eq!(
        pruned.ops_applied, 6,
        "api node, new fallback link, two link deletes, contains create, contains delete"
    );

    let pruned_set = load_link_set(&link_set_path(cache_dir.path())).expect("load link set");
    assert_eq!(pruned_set.edges.len(), 2);
    assert!(
        pruned_set
            .edges
            .iter()
            .any(|edge| edge.kind == RelationKind::PublishTo),
        "publish edge survives"
    );
    let fallback = pruned_set
        .edges
        .iter()
        .find(|edge| edge.kind == RelationKind::CallsApi)
        .expect("fallback calls edge");
    assert_eq!(fallback.from, CanonicalId::from("cons.caller"));
    assert_eq!(pruned_set.resources.len(), 1);
    assert_eq!(fallback.to, pruned_set.resources[0].id);
    assert_eq!(pruned_set.resources[0].resource_type, "api");
    assert_eq!(pruned_set.topics.len(), 1);
    assert_eq!(pruned_set.topics[0].commits.len(), 1);
    assert_eq!(store.edge_count(), 4, "stale links removed from the store");
    assert_eq!(store.node_count(), 2, "topic plus the api resource node");
}

#[tokio::test]
async fn link_pass_without_index_state_touches_nothing() {
    let cache_dir = tempfile::tempdir().expect("tempdir");
    let workspace = workspace();
    let state = IndexState::default();
    seed_contracts(cache_dir.path());

    let store = MockStore::new();
    let report = run_link_pass(&store, &workspace, cache_dir.path(), &state)
        .await
        .expect("link pass");
    assert_eq!(report.ops_applied, 0);
    assert_eq!(report.edges, 0);
    assert!(
        !link_set_path(cache_dir.path()).exists(),
        "no link set is written while nothing was indexed"
    );
    assert_eq!(store.edge_count(), 0);
    assert_eq!(state.last_sha("prov", "main"), None);
}
