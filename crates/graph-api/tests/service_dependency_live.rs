use std::sync::Arc;

use ckg_domain::{CanonicalId, GraphEdge, GraphNode, NodeKind, RelationKind, SourceLocation};
use ckg_graph_api::{DeltaOp, GraphApi, GraphDelta, GraphStore};
use ckg_neo4j_store::Neo4jStore;

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect_or_skip() -> Option<Neo4jStore> {
    let uri = match std::env::var("NEO4J_URI") {
        Ok(uri) if !uri.is_empty() => uri,
        _ => {
            eprintln!("skipping: NEO4J_URI not set");
            return None;
        }
    };
    let user = std::env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".into());
    let password = std::env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "testpassword".into());
    let database = std::env::var("NEO4J_DATABASE").unwrap_or_else(|_| "neo4j".into());
    match Neo4jStore::connect(&uri, &user, &password, &database) {
        Ok(store) => match store.health().await {
            Ok(()) => Some(store),
            Err(err) => {
                eprintln!("skipping: neo4j unhealthy: {err}");
                None
            }
        },
        Err(err) => {
            eprintln!("skipping: cannot connect to neo4j at {uri}: {err:#}");
            None
        }
    }
}

fn symbol(id: &str, name: &str) -> GraphNode {
    GraphNode::new(NodeKind::Function, CanonicalId::from(id), name).with_location(
        SourceLocation::new("org/links", "sha-main", "src/lib.rs", 1, 0, 2, 0),
    )
}

async fn seed_service_dependencies(store: &Neo4jStore) {
    let mut ops = vec![
        DeltaOp::CreateNode(GraphNode::new(
            NodeKind::Branch,
            CanonicalId::from("br-main"),
            "main",
        )),
        DeltaOp::CreateNode(GraphNode::new(
            NodeKind::Branch,
            CanonicalId::from("br-dev"),
            "develop",
        )),
        DeltaOp::CreateNode(GraphNode::new(
            NodeKind::Commit,
            CanonicalId::from("commit-main"),
            "sha-main",
        )),
        DeltaOp::CreateNode(GraphNode::new(
            NodeKind::Commit,
            CanonicalId::from("commit-dev"),
            "sha-dev",
        )),
        DeltaOp::CreateRelationship(GraphEdge::new(
            RelationKind::PointsTo,
            CanonicalId::from("br-main"),
            CanonicalId::from("commit-main"),
        )),
        DeltaOp::CreateRelationship(GraphEdge::new(
            RelationKind::PointsTo,
            CanonicalId::from("br-dev"),
            CanonicalId::from("commit-dev"),
        )),
        DeltaOp::CreateNode(symbol("live-consumer", "live_consumer")),
        DeltaOp::CreateNode(symbol("live-provider", "live_provider")),
        DeltaOp::CreateNode(GraphNode::new(
            NodeKind::Topic,
            CanonicalId::from("live-queue"),
            "billing.q",
        )),
    ];
    for node_id in ["live-consumer", "live-provider", "live-queue"] {
        for commit in ["commit-main", "commit-dev"] {
            ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                CanonicalId::from(commit),
                CanonicalId::from(node_id),
            )));
        }
    }
    ops.push(DeltaOp::CreateRelationship(
        GraphEdge::new(
            RelationKind::CallsApi,
            CanonicalId::from("live-consumer"),
            CanonicalId::from("live-provider"),
        )
        .with_property("branch_pairs", serde_json::json!(["main"])),
    ));
    ops.push(DeltaOp::CreateRelationship(GraphEdge::new(
        RelationKind::ConsumeFrom,
        CanonicalId::from("live-consumer"),
        CanonicalId::from("live-queue"),
    )));
    let delta = GraphDelta {
        ops,
        base_revision: None,
        target_revision: None,
    };
    let report = store.apply_delta(&delta).await.expect("seed failed");
    assert!(report.is_success(), "seed errors: {:?}", report.errors);
}

#[tokio::test]
async fn live_service_dependency_edges_with_branch_pairs() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");
    seed_service_dependencies(&store).await;

    let store = Arc::new(store);
    let api = GraphApi::with_defaults(store.clone());
    let consumer = CanonicalId::from("live-consumer");
    let provider = CanonicalId::from("live-provider");

    let all = api.get_dependencies(&consumer, 10, None).await.unwrap();
    let kinds: Vec<RelationKind> = all.data.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![RelationKind::CallsApi, RelationKind::ConsumeFrom]
    );
    let calls = &all.data[0];
    assert_eq!(
        calls.properties.get("branch_pairs"),
        Some(&serde_json::json!(["main"])),
        "edge properties must round-trip through neo4j"
    );

    let on_main = api
        .get_dependencies(&consumer, 10, Some("main"))
        .await
        .unwrap();
    assert_eq!(on_main.data.len(), 2);

    let on_dev = api
        .get_dependencies(&consumer, 10, Some("develop"))
        .await
        .unwrap();
    assert_eq!(on_dev.data.len(), 1, "branch_pairs excludes develop");
    assert_eq!(on_dev.data[0].kind, RelationKind::ConsumeFrom);

    let dependents_main = api
        .get_dependents(&provider, 10, Some("main"))
        .await
        .unwrap();
    assert_eq!(dependents_main.data.len(), 1);

    let dependents_dev = api
        .get_dependents(&provider, 10, Some("develop"))
        .await
        .unwrap();
    assert!(dependents_dev.data.is_empty());

    store.reset().await.expect("reset failed");
}
