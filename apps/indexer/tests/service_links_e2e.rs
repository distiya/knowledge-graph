use std::path::Path;
use std::process::Command;

use ckg_domain::{CanonicalId, GraphEdge, NodeKind, RelationKind};
use ckg_graph_api::{GraphStore, MockStore};
use ckg_indexer::jobs::run_index_pass;
use ckg_links::{link_set_path, load_link_set};
use ckg_repository_config::{RepositoryConfig, WorkspaceConfig};

const PROVIDER_V1: &str = r#"const express = require("express");
const { Kafka } = require("kafkajs");
const app = express();

function getOrder(req, res) {
  res.json({ id: req.params.id });
}

app.get("/orders/:id", getOrder);

async function watchBilling(consumer) {
  await consumer.subscribe({ topic: "billing.ready" });
}
"#;

const PROVIDER_V2: &str = r#"const express = require("express");
const { Kafka } = require("kafkajs");
const app = express();

function getOrder(req, res) {
  res.json({ id: req.params.id });
}

async function watchBilling(consumer) {
  await consumer.subscribe({ topic: "billing.ready" });
}
"#;

const PROVIDER_V3: &str = r#"const express = require("express");
const { Kafka } = require("kafkajs");
const app = express();

function getOrder(req, res) {
  res.json({ id: req.params.id });
}
"#;

const CONSUMER_V1: &str = r#"const { Kafka } = require("kafkajs");

async function charge() {
  await fetch("https://api.orders.internal/orders/42", { method: "GET" });
}

async function announce(producer) {
  await producer.send({ topic: "billing.ready", messages: [] });
}
"#;

const CONSUMER_V2: &str = r#"const { Kafka } = require("kafkajs");

async function charge() {
  await fetch("https://api.orders.internal/orders/42", { method: "GET" });
}
"#;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .expect("git CLI must be available to run these tests");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-b", "main"]);
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

fn write(path: &Path, content: &str) {
    std::fs::write(path, content).expect("write file");
}

async fn find_by_name(store: &MockStore, name: &str) -> Option<CanonicalId> {
    let nodes = store.find_nodes_by_name(name, 10).await.expect("find");
    nodes.into_iter().find(|n| n.name == name).map(|n| n.id)
}

async fn edges_from(store: &MockStore, id: &CanonicalId, kind: RelationKind) -> Vec<GraphEdge> {
    store
        .get_edges_from(id, Some(kind), 50)
        .await
        .expect("edges from")
}

async fn edges_to(store: &MockStore, id: &CanonicalId, kind: RelationKind) -> Vec<GraphEdge> {
    store
        .get_edges_to(id, Some(kind), 50)
        .await
        .expect("edges to")
}

async fn run_pass(store: &MockStore, config: &Path, cache: &Path) {
    let outcome = run_index_pass(store, config, cache, None, None, false)
        .await
        .expect("index pass");
    assert!(
        outcome.failures.is_empty(),
        "pass failures: {:?}",
        outcome.failures
    );
}

#[tokio::test]
async fn cross_repo_service_edges_follow_code_changes() {
    let provider_dir = tempfile::tempdir().expect("provider tempdir");
    let consumer_dir = tempfile::tempdir().expect("consumer tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let config_dir = tempfile::tempdir().expect("config tempdir");

    std::fs::create_dir_all(provider_dir.path().join("src")).unwrap();
    std::fs::create_dir_all(consumer_dir.path().join("src")).unwrap();
    let api_js = provider_dir.path().join("src/api.js");
    let client_js = consumer_dir.path().join("src/client.js");
    write(&api_js, PROVIDER_V1);
    write(&client_js, CONSUMER_V1);
    init_repo(provider_dir.path());
    init_repo(consumer_dir.path());

    let mut workspace = WorkspaceConfig {
        repositories: vec![
            RepositoryConfig::new(
                "orders-svc",
                "acme/orders-svc",
                "https://example.com/orders-svc.git",
            )
            .with_branches(["main"])
            .with_local_path(provider_dir.path().to_string_lossy().as_ref()),
            RepositoryConfig::new(
                "billing-svc",
                "acme/billing-svc",
                "https://example.com/billing-svc.git",
            )
            .with_branches(["main"])
            .with_local_path(consumer_dir.path().to_string_lossy().as_ref()),
        ],
        default_branches: vec![],
        ..Default::default()
    };
    workspace.links.base_urls.insert(
        "https://api.orders.internal".to_string(),
        "orders-svc".to_string(),
    );
    let config_path = config_dir.path().join("workspace.toml");
    workspace.save(&config_path).expect("save config");

    let store = MockStore::new();
    run_pass(&store, &config_path, cache_dir.path()).await;

    let set = load_link_set(&link_set_path(cache_dir.path())).expect("load link set");
    assert_eq!(
        set.edges.len(),
        3,
        "one CALLS_API + one PUBLISH_TO + one CONSUME_FROM edge"
    );
    assert_eq!(set.topics.len(), 1, "one shared topic hub");

    let charge = find_by_name(&store, "charge").await.expect("charge node");
    let get_order = find_by_name(&store, "getOrder")
        .await
        .expect("getOrder node");
    let announce = find_by_name(&store, "announce")
        .await
        .expect("announce node");
    let watch = find_by_name(&store, "watchBilling")
        .await
        .expect("watchBilling node");
    let topic = find_by_name(&store, "billing.ready")
        .await
        .expect("topic node");

    let topic_node = store.get_node(&topic).await.expect("topic lookup");
    assert_eq!(
        topic_node.map(|n| n.kind),
        Some(NodeKind::Topic),
        "topic hub is a Topic node"
    );

    let calls = edges_from(&store, &charge, RelationKind::CallsApi).await;
    assert_eq!(calls.len(), 1, "one CALLS_API edge from charge");
    assert_eq!(calls[0].to, get_order, "CALLS_API points at the handler");
    assert_eq!(
        calls[0].properties.get("confidence"),
        Some(&serde_json::json!("high")),
        "base_urls restriction yields high confidence"
    );
    assert!(
        calls[0]
            .properties
            .get("branch_pairs")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty()),
        "CALLS_API carries branch_pairs"
    );

    let publish = edges_from(&store, &announce, RelationKind::PublishTo).await;
    assert_eq!(publish.len(), 1, "publisher links to the topic hub");
    assert_eq!(publish[0].to, topic);
    let consume = edges_from(&store, &watch, RelationKind::ConsumeFrom).await;
    assert_eq!(consume.len(), 1, "subscriber links to the topic hub");
    assert_eq!(consume[0].to, topic);

    // Provider drops the route: the REST edge must vanish, messaging stays.
    // The consumer's fetch no longer matches a provider, so it falls back to
    // an external `CallsApi -> Resource(api)` edge.
    write(&api_js, PROVIDER_V2);
    commit_all(provider_dir.path(), "drop route");
    run_pass(&store, &config_path, cache_dir.path()).await;
    let calls_after = edges_from(&store, &charge, RelationKind::CallsApi).await;
    assert_eq!(
        calls_after.len(),
        1,
        "unmatched fetch falls back to an api resource edge"
    );
    assert_ne!(
        calls_after[0].to, get_order,
        "CALLS_API no longer points at the handler"
    );
    let api_node = store
        .get_node(&calls_after[0].to)
        .await
        .expect("api lookup")
        .expect("api resource node exists");
    assert_eq!(
        api_node.kind,
        NodeKind::Resource,
        "fallback target is a Resource"
    );
    assert_eq!(
        api_node.properties.get("resource_type"),
        Some(&serde_json::json!("api")),
        "resource_type is api"
    );
    assert_eq!(
        calls_after[0].properties.get("confidence"),
        Some(&serde_json::json!("low")),
        "unmatched fallback ranks low"
    );
    assert_eq!(
        edges_to(&store, &topic, RelationKind::ConsumeFrom)
            .await
            .len(),
        1,
        "subscription still live"
    );
    assert!(
        store.get_node(&topic).await.expect("topic").is_some(),
        "topic retained"
    );

    // Provider drops the subscription: consume edge gone, topic kept by publisher.
    write(&api_js, PROVIDER_V3);
    commit_all(provider_dir.path(), "drop subscription");
    run_pass(&store, &config_path, cache_dir.path()).await;
    assert!(
        edges_to(&store, &topic, RelationKind::ConsumeFrom)
            .await
            .is_empty(),
        "stale CONSUME_FROM edge removed"
    );
    assert_eq!(
        edges_to(&store, &topic, RelationKind::PublishTo)
            .await
            .len(),
        1,
        "publisher still linked"
    );
    assert!(
        store.get_node(&topic).await.expect("topic").is_some(),
        "topic retained while a publisher exists"
    );

    // Consumer drops the publisher: the topic is unreferenced and GC'd.
    write(&client_js, CONSUMER_V2);
    commit_all(consumer_dir.path(), "drop publisher");
    run_pass(&store, &config_path, cache_dir.path()).await;
    assert!(
        edges_to(&store, &topic, RelationKind::PublishTo)
            .await
            .is_empty(),
        "stale PUBLISH_TO edge removed"
    );
    assert!(
        store
            .get_node(&topic)
            .await
            .expect("topic lookup")
            .is_none(),
        "unreferenced topic hub deleted"
    );

    let set = load_link_set(&link_set_path(cache_dir.path())).expect("load link set");
    assert_eq!(
        set.edges.len(),
        1,
        "only the unmatched api fallback edge remains"
    );
    assert_eq!(set.edges[0].kind, RelationKind::CallsApi);
    assert_eq!(
        set.resources.len(),
        1,
        "the external api resource node remains"
    );
    assert_eq!(set.resources[0].resource_type, "api");
    assert!(set.topics.is_empty(), "no topics remain in the link set");
    assert!(
        store
            .get_node(&topic)
            .await
            .expect("topic lookup")
            .is_none(),
        "no topic node remains in the store"
    );
}
