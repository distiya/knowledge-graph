use std::path::Path;
use std::process::Command;

use ckg_domain::{CanonicalId, GraphEdge, NodeKind, RelationKind};
use ckg_graph_api::{GraphStore, MockStore};
use ckg_indexer::jobs::run_index_pass;
use ckg_links::{link_set_path, load_link_set};
use ckg_repository_config::{RepositoryConfig, WorkspaceConfig};

const SYNC_V1: &str = r#"import os

import boto3
from google.cloud import bigquery
from sqlalchemy import create_engine


def export_orders():
    create_engine("postgresql://orders:secret@db.internal:5432/orders")


def copy_exports():
    s3 = boto3.client("s3")
    s3.put_object(Bucket="orders-exports", Key="a.json", Body=b"x")
    s3.get_object(Bucket="orders-exports", Key="a.json")
    s3.put_object(Bucket="legacy-exports", Key="k", Body=b"y")


def trigger_worker():
    client = boto3.client("lambda")
    client.invoke(FunctionName="billing-worker", Payload=b"{}")


def refresh_metrics():
    bq = bigquery.Client()
    bq.query("SELECT * FROM `analytics-proj.orders.events` WHERE x = 1")


def connect_reports():
    url = os.environ["REPORTS_DB_URL"]
    worker = os.getenv("WORKER_FUNCTION_NAME")
"#;

const SYNC_V2: &str = r#"import os

import boto3
from sqlalchemy import create_engine


def export_orders():
    create_engine("postgresql://orders:secret@db.internal:5432/orders")


def copy_exports():
    s3 = boto3.client("s3")
    s3.put_object(Bucket="orders-exports", Key="a.json", Body=b"x")
    s3.get_object(Bucket="orders-exports", Key="a.json")
    s3.put_object(Bucket="legacy-exports", Key="k", Body=b"y")


def trigger_worker():
    client = boto3.client("lambda")
    client.invoke(FunctionName="billing-worker", Payload=b"{}")


def connect_reports():
    url = os.environ["REPORTS_DB_URL"]
    worker = os.getenv("WORKER_FUNCTION_NAME")
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

fn write(path: &Path, content: &str) {
    std::fs::write(path, content).expect("write file");
}

async fn find_by_name(store: &MockStore, name: &str) -> Option<CanonicalId> {
    let nodes = store.find_nodes_by_name(name, 10).await.expect("find");
    nodes.into_iter().find(|n| n.name == name).map(|n| n.id)
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
async fn external_resources_emit_every_dependency_kind_and_gc() {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let config_dir = tempfile::tempdir().expect("config tempdir");

    std::fs::create_dir_all(repo_dir.path().join("src")).unwrap();
    let sync_py = repo_dir.path().join("src/sync.py");
    let env_file = repo_dir.path().join(".env");
    write(&sync_py, SYNC_V1);
    write(
        &env_file,
        "REPORTS_DB_URL=postgresql://reports.internal:5432/reports\n",
    );

    git(repo_dir.path(), &["init", "-q"]);
    git(repo_dir.path(), &["checkout", "-q", "-b", "main"]);
    git(repo_dir.path(), &["add", "-A"]);
    git(repo_dir.path(), &["commit", "-q", "-m", "init"]);

    let mut workspace = WorkspaceConfig {
        repositories: vec![
            RepositoryConfig::new(
                "data-pipeline",
                "acme/data-pipeline",
                "https://example.com/data-pipeline.git",
            )
            .with_branches(["main"])
            .with_local_path(repo_dir.path().to_string_lossy().as_ref()),
        ],
        default_branches: vec![],
        ..Default::default()
    };
    workspace.links.resources.insert(
        "legacy-exports".to_string(),
        "bucket:s3://legacy-prod-exports".to_string(),
    );
    let config_path = config_dir.path().join("workspace.toml");
    workspace.save(&config_path).expect("save config");

    let store = MockStore::new();
    run_pass(&store, &config_path, cache_dir.path()).await;

    let set = load_link_set(&link_set_path(cache_dir.path())).expect("load link set");
    assert_eq!(
        set.resources.len(),
        7,
        "resources: {:?}",
        set.resources
            .iter()
            .map(|r| (r.resource_type.as_str(), r.identity.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(set.edges.len(), 8, "resource edges: {:?}", set.edges);

    // Literal DSN: high confidence, credentials and default port stripped.
    let orders_db = find_by_name(&store, "postgres://db.internal/orders")
        .await
        .expect("literal database node");
    let connect = edges_to(&store, &orders_db, RelationKind::ConnectsTo).await;
    assert_eq!(connect.len(), 1, "one CONNECTS_TO to the literal dsn");
    assert_eq!(
        connect[0].properties.get("confidence"),
        Some(&serde_json::json!("high")),
        "url literal ranks high"
    );
    assert_eq!(
        connect[0].properties.get("resource_type"),
        Some(&serde_json::json!("database"))
    );

    // .env evidence resolves the alias: medium confidence with evidence.
    let reports_db = find_by_name(&store, "postgres://reports.internal/reports")
        .await
        .expect("env-evidenced database node");
    let connect = edges_to(&store, &reports_db, RelationKind::ConnectsTo).await;
    assert_eq!(connect.len(), 1, "one CONNECTS_TO to the env-evidenced dsn");
    assert_eq!(
        connect[0].properties.get("confidence"),
        Some(&serde_json::json!("medium")),
        "evidence-resolved alias ranks medium"
    );
    let evidence = connect[0]
        .properties
        .get("evidence")
        .and_then(|v| v.as_array())
        .expect("evidence array");
    assert_eq!(evidence.len(), 2, "env + evidence entries: {evidence:?}");
    assert_eq!(evidence[0]["kind"], serde_json::json!("env"));
    assert_eq!(evidence[0]["detail"], serde_json::json!("REPORTS_DB_URL"));
    assert_eq!(evidence[1]["kind"], serde_json::json!("evidence"));

    // Bucket: s3 put/get produce WRITE + READ edges against one node.
    let bucket = find_by_name(&store, "orders-exports")
        .await
        .expect("bucket node");
    let writes = edges_to(&store, &bucket, RelationKind::WritesTo).await;
    assert_eq!(writes.len(), 1, "one WRITES_TO from put_object");
    assert_eq!(
        writes[0].properties.get("confidence"),
        Some(&serde_json::json!("medium")),
        "bare bucket name ranks medium"
    );
    let reads = edges_to(&store, &bucket, RelationKind::ReadsFrom).await;
    assert_eq!(reads.len(), 1, "one READS_FROM from get_object");

    // Registry entry overrides the extracted name: high confidence.
    let legacy = find_by_name(&store, "legacy-prod-exports")
        .await
        .expect("registry bucket node");
    let writes = edges_to(&store, &legacy, RelationKind::WritesTo).await;
    assert_eq!(writes.len(), 1, "registry bucket edge");
    assert_eq!(
        writes[0].properties.get("confidence"),
        Some(&serde_json::json!("high")),
        "registry resolution ranks high"
    );
    let evidence = writes[0]
        .properties
        .get("evidence")
        .and_then(|v| v.as_array())
        .expect("evidence array");
    assert_eq!(evidence[0]["kind"], serde_json::json!("registry"));
    assert_eq!(evidence[0]["detail"], serde_json::json!("legacy-exports"));

    // Lambda invoke.
    let function = find_by_name(&store, "billing-worker")
        .await
        .expect("function node");
    let invokes = edges_to(&store, &function, RelationKind::Invokes).await;
    assert_eq!(invokes.len(), 1, "one INVOKES to the lambda");
    assert_eq!(
        invokes[0].properties.get("mechanism"),
        Some(&serde_json::json!("lambda"))
    );

    // BigQuery query.
    let table = find_by_name(&store, "analytics-proj.orders.events")
        .await
        .expect("table node");
    let queries = edges_to(&store, &table, RelationKind::Queries).await;
    assert_eq!(queries.len(), 1, "one QUERIES to the bigquery table");
    assert_eq!(
        queries[0].properties.get("mechanism"),
        Some(&serde_json::json!("bigquery"))
    );

    // Unresolved env alias: low-confidence placeholder node.
    let alias = find_by_name(&store, "env:WORKER_FUNCTION_NAME")
        .await
        .expect("unresolved env placeholder node");
    let connect = edges_to(&store, &alias, RelationKind::ConnectsTo).await;
    assert_eq!(connect.len(), 1, "one edge to the env placeholder");
    assert_eq!(
        connect[0].properties.get("confidence"),
        Some(&serde_json::json!("low")),
        "unresolved alias ranks low"
    );

    for id in [
        &orders_db,
        &reports_db,
        &bucket,
        &legacy,
        &function,
        &table,
        &alias,
    ] {
        let node = store
            .get_node(id)
            .await
            .expect("node lookup")
            .expect("node exists");
        assert_eq!(node.kind, NodeKind::Resource, "all targets are resources");
        assert!(
            node.properties.get("resource_type").is_some(),
            "resource_type property present"
        );
        let contains = store
            .get_edges_to(id, Some(RelationKind::ContainsStateOf), 10)
            .await
            .expect("contains edges");
        assert!(
            !contains.is_empty(),
            "resource is contained by a branch commit: {id}"
        );
    }

    // Drop the BigQuery code: the table edge and node are garbage-collected.
    write(&sync_py, SYNC_V2);
    git(repo_dir.path(), &["add", "-A"]);
    git(repo_dir.path(), &["commit", "-q", "-m", "drop bigquery"]);
    run_pass(&store, &config_path, cache_dir.path()).await;

    assert!(
        edges_to(&store, &table, RelationKind::Queries)
            .await
            .is_empty(),
        "stale QUERIES edge removed"
    );
    assert!(
        store
            .get_node(&table)
            .await
            .expect("table lookup")
            .is_none(),
        "unreferenced table resource deleted"
    );
    for id in [&orders_db, &reports_db, &bucket, &legacy, &function, &alias] {
        assert!(
            store.get_node(id).await.expect("node lookup").is_some(),
            "referenced resource retained"
        );
    }

    let set = load_link_set(&link_set_path(cache_dir.path())).expect("load link set");
    assert_eq!(
        set.resources.len(),
        6,
        "table resource dropped from the link set"
    );
    assert_eq!(set.edges.len(), 7, "queries edge dropped from the link set");
}
