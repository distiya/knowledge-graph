use ckg_domain::{
    CanonicalId, GraphEdge, GraphNode, NodeKind, Provenance, ProvenanceSource, RelationKind,
    SourceLocation,
};
use ckg_graph_delta::{DeltaBuilder, DeltaOp};
use ckg_neo4j_store::{
    AnalyticsEngine, AnalyticsMode, AnalyticsSpec, ApplyReport, GraphStore, Neo4jStore,
};

// All tests share one database and reset() wipes it, so serialize them.
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
            eprintln!("skipping: cannot connect to neo4j at {uri}: {err}");
            None
        }
    }
}

fn function_node(id: &str, name: &str, hash: &str) -> GraphNode {
    GraphNode::new(NodeKind::Function, CanonicalId::from(id), name)
        .with_qualified_name(format!("pkg::{name}"))
        .with_language("rust")
        .with_content_hash(hash)
        .with_location(SourceLocation::new(
            "org/svc",
            "abc123",
            "src/lib.rs",
            1,
            1,
            10,
            2,
        ))
        .with_provenance(Provenance::from(ProvenanceSource::TreeSitter))
}

fn calls_edge(from: &str, to: &str, hash: &str) -> GraphEdge {
    GraphEdge::new(
        RelationKind::Calls,
        CanonicalId::from(from),
        CanonicalId::from(to),
    )
    .with_content_hash(hash)
}

#[tokio::test]
async fn health_skips_gracefully_without_server() {
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.health().await.expect("health check failed");
}

#[tokio::test]
async fn apply_delta_roundtrip_and_queries() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let a = function_node("node-a", "alpha", "hash-a");
    let b = function_node("node-b", "beta", "hash-b");
    let edge = calls_edge("node-a", "node-b", "edge-1");

    let delta = DeltaBuilder::new()
        .base_revision("r0")
        .target_revision("r1")
        .create_node(a.clone())
        .create_node(b.clone())
        .create_relationship(edge.clone())
        .build();

    let report = store.apply_delta(&delta).await.expect("apply failed");
    assert_eq!(
        report,
        ApplyReport {
            created_nodes: 2,
            updated_nodes: 0,
            deleted_nodes: 0,
            created_rels: 1,
            deleted_rels: 0,
            nodes_retained: 0,
            errors: vec![],
        }
    );

    let fetched = store
        .get_node(&CanonicalId::from("node-a"))
        .await
        .expect("get_node failed");
    let fetched = fetched.expect("node-a missing");
    assert_eq!(fetched.id, a.id);
    assert_eq!(fetched.kind, NodeKind::Function);
    assert_eq!(fetched.name, "alpha");
    assert_eq!(fetched.qualified_name, "pkg::alpha");
    assert_eq!(fetched.language.as_deref(), Some("rust"));
    assert_eq!(fetched.content_hash.as_deref(), Some("hash-a"));
    assert_eq!(
        fetched.location.as_ref().map(|l| l.path.as_str()),
        Some("src/lib.rs")
    );
    assert_eq!(fetched.provenance.len(), 1);

    let by_name = store
        .find_nodes_by_name("alpha", 10)
        .await
        .expect("find failed");
    assert_eq!(by_name.len(), 1);
    assert_eq!(by_name[0].id, a.id);

    let from_edges = store
        .get_edges_from(&CanonicalId::from("node-a"), None, 10)
        .await
        .expect("get_edges_from failed");
    assert_eq!(from_edges.len(), 1);
    assert_eq!(from_edges[0].kind, RelationKind::Calls);
    assert_eq!(from_edges[0].from.as_str(), "node-a");
    assert_eq!(from_edges[0].to.as_str(), "node-b");
    assert_eq!(from_edges[0].content_hash.as_deref(), Some("edge-1"));

    let typed = store
        .get_edges_from(&CanonicalId::from("node-a"), Some(RelationKind::Calls), 10)
        .await
        .expect("typed get_edges_from failed");
    assert_eq!(typed.len(), 1);

    let none_typed = store
        .get_edges_from(
            &CanonicalId::from("node-a"),
            Some(RelationKind::Imports),
            10,
        )
        .await
        .expect("typed get_edges_from failed");
    assert!(none_typed.is_empty());

    let to_edges = store
        .get_edges_to(&CanonicalId::from("node-b"), None, 10)
        .await
        .expect("get_edges_to failed");
    assert_eq!(to_edges.len(), 1);
    assert_eq!(to_edges[0].from.as_str(), "node-a");

    let mut updated_a = a.clone();
    updated_a.content_hash = Some("hash-a-v2".into());
    updated_a
        .properties
        .insert("loc".into(), serde_json::json!(7));

    let update_delta = DeltaBuilder::new().update_node(updated_a.clone()).build();
    let report = store
        .apply_delta(&update_delta)
        .await
        .expect("update failed");
    assert_eq!(report.updated_nodes, 1);
    assert_eq!(report.errors, Vec::<String>::new());

    let refetched = store
        .get_node(&CanonicalId::from("node-a"))
        .await
        .expect("get_node failed")
        .expect("node-a missing");
    assert_eq!(refetched.content_hash.as_deref(), Some("hash-a-v2"));
    assert_eq!(refetched.properties.get("loc"), Some(&serde_json::json!(7)));

    let mut rediff = DeltaBuilder::new()
        .delete_relationship(RelationKind::Calls, "node-a", "node-b")
        .delete_node("node-b")
        .delete_node("node-a")
        .build();
    rediff.base_revision = Some("r1".into());
    let report = store.apply_delta(&rediff).await.expect("delete failed");
    assert_eq!(report.deleted_rels, 1);
    assert_eq!(report.deleted_nodes, 2);
    assert_eq!(report.errors, Vec::<String>::new());

    assert!(
        store
            .get_node(&CanonicalId::from("node-a"))
            .await
            .expect("get_node failed")
            .is_none()
    );
    assert!(
        store
            .get_edges_from(&CanonicalId::from("node-a"), None, 10)
            .await
            .expect("get_edges_from failed")
            .is_empty()
    );

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn guarded_delete_retains_node_until_last_contains_state_of_removed() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let symbol = function_node("shared-sym", "shared_sym", "h1");
    let commit_a = GraphNode::new(NodeKind::Commit, CanonicalId::from("commit-a"), "sha-a");
    let commit_b = GraphNode::new(NodeKind::Commit, CanonicalId::from("commit-b"), "sha-b");
    let branch_b = GraphNode::new(NodeKind::Branch, CanonicalId::from("branch-b"), "main");

    let seed = DeltaBuilder::new()
        .create_node(symbol.clone())
        .create_node(commit_a.clone())
        .create_node(commit_b.clone())
        .create_node(branch_b.clone())
        .create_relationship(GraphEdge::new(
            RelationKind::ContainsStateOf,
            CanonicalId::from("commit-a"),
            CanonicalId::from("shared-sym"),
        ))
        .create_relationship(GraphEdge::new(
            RelationKind::ContainsStateOf,
            CanonicalId::from("commit-b"),
            CanonicalId::from("shared-sym"),
        ))
        .create_relationship(GraphEdge::new(
            RelationKind::PointsTo,
            CanonicalId::from("branch-b"),
            CanonicalId::from("commit-b"),
        ))
        .build();
    store.apply_delta(&seed).await.expect("seed failed");

    let partial = DeltaBuilder::new()
        .delete_relationship(RelationKind::ContainsStateOf, "commit-a", "shared-sym")
        .delete_node("shared-sym")
        .build();
    let report = store.apply_delta(&partial).await.expect("partial delete");
    assert_eq!(report.deleted_nodes, 0);
    assert_eq!(report.nodes_retained, 1);

    let retained = store
        .get_node(&CanonicalId::from("shared-sym"))
        .await
        .expect("get")
        .expect("node must be retained while commit-b still contains it");
    assert_eq!(retained.id, symbol.id);
    let remaining = store
        .get_edges_to(&CanonicalId::from("shared-sym"), None, 10)
        .await
        .expect("edges");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].from.as_str(), "commit-b");

    let final_delete = DeltaBuilder::new()
        .delete_relationship(RelationKind::ContainsStateOf, "commit-b", "shared-sym")
        .delete_node("shared-sym")
        .build();
    let report = store
        .apply_delta(&final_delete)
        .await
        .expect("final delete");
    assert_eq!(report.deleted_nodes, 1);
    assert_eq!(report.nodes_retained, 0);
    assert!(
        store
            .get_node(&CanonicalId::from("shared-sym"))
            .await
            .expect("get")
            .is_none(),
        "node must be gone once no ContainsStateOf remains"
    );

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn apply_delta_is_idempotent_against_store() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let node = function_node("idem-node", "idem", "h1");
    let delta = DeltaBuilder::new().create_node(node.clone()).build();

    let first = store.apply_delta(&delta).await.expect("apply failed");
    assert_eq!(first.created_nodes, 1);

    let second = store.apply_delta(&delta).await.expect("apply failed");
    assert_eq!(second.created_nodes, 0);
    assert_eq!(second.updated_nodes, 1);
    assert!(second.errors.is_empty());

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn missing_relationship_endpoints_reported() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let delta = DeltaBuilder::new()
        .create_relationship(calls_edge("ghost-a", "ghost-b", "h"))
        .build();
    let report = store.apply_delta(&delta).await.expect("apply failed");
    assert_eq!(report.created_rels, 0);
    assert_eq!(report.errors.len(), 1);
    assert!(report.errors[0].contains("endpoint"));

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn diff_then_apply_incremental_change() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let old_nodes = ckg_graph_delta::NodeMap::from([(
        "inc-node".to_string(),
        function_node("inc-node", "inc", "h1"),
    )]);
    let old_edges = ckg_graph_delta::EdgeMap::new();

    let mut new_node = function_node("inc-node", "inc", "h2");
    new_node
        .properties
        .insert("version".into(), serde_json::json!(2));
    let new_nodes = ckg_graph_delta::NodeMap::from([("inc-node".to_string(), new_node.clone())]);

    store
        .apply_delta(
            &DeltaBuilder::new()
                .create_node(old_nodes["inc-node"].clone())
                .build(),
        )
        .await
        .expect("seed failed");

    let delta = ckg_graph_delta::diff_graphs(&old_nodes, &new_nodes, &old_edges, &old_edges);
    assert_eq!(delta.ops.len(), 1);
    assert!(matches!(delta.ops[0], DeltaOp::UpdateNode(_)));

    let report = store.apply_delta(&delta).await.expect("apply failed");
    assert_eq!(report.updated_nodes, 1);

    let stored = store
        .get_node(&CanonicalId::from("inc-node"))
        .await
        .expect("get failed")
        .expect("node missing");
    assert_eq!(stored.content_hash.as_deref(), Some("h2"));

    store.reset().await.expect("reset failed");
}

#[tokio::test]
#[ignore = "heavy: bulk write benchmark, run with --ignored"]
async fn heavy_bulk_delta() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let mut builder = DeltaBuilder::new();
    for i in 0..500 {
        builder = builder.create_node(function_node(
            &format!("bulk-{i}"),
            &format!("fn_{i}"),
            &format!("hash-{i}"),
        ));
    }
    for i in 0..499 {
        builder = builder.create_relationship(calls_edge(
            &format!("bulk-{i}"),
            &format!("bulk-{}", i + 1),
            &format!("e-{i}"),
        ));
    }
    let report = store
        .apply_delta(&builder.build())
        .await
        .expect("apply failed");
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
    assert_eq!(report.created_nodes, 500);
    assert_eq!(report.created_rels, 499);

    let edges = store
        .get_edges_from(&CanonicalId::from("bulk-0"), None, 10)
        .await
        .expect("edges failed");
    assert_eq!(edges.len(), 1);

    store.reset().await.expect("reset failed");
}

async fn seed_dependency_graph(store: &Neo4jStore) {
    let app = GraphNode::new(NodeKind::Function, CanonicalId::from("an-app"), "app_fn");
    let worker = GraphNode::new(
        NodeKind::Function,
        CanonicalId::from("an-worker"),
        "worker_fn",
    );
    let db = GraphNode::new(
        NodeKind::Resource,
        CanonicalId::from("an-db"),
        "postgres://orders-db",
    );
    let bucket = GraphNode::new(
        NodeKind::Resource,
        CanonicalId::from("an-bucket"),
        "s3://orders-exports",
    );
    let delta = DeltaBuilder::new()
        .create_node(app)
        .create_node(worker)
        .create_node(db)
        .create_node(bucket)
        .create_relationship(GraphEdge::new(
            RelationKind::ConnectsTo,
            CanonicalId::from("an-app"),
            CanonicalId::from("an-db"),
        ))
        .create_relationship(GraphEdge::new(
            RelationKind::ReadsFrom,
            CanonicalId::from("an-app"),
            CanonicalId::from("an-db"),
        ))
        .create_relationship(GraphEdge::new(
            RelationKind::DependsOn,
            CanonicalId::from("an-worker"),
            CanonicalId::from("an-app"),
        ))
        .create_relationship(GraphEdge::new(
            RelationKind::WritesTo,
            CanonicalId::from("an-app"),
            CanonicalId::from("an-bucket"),
        ))
        .build();
    let report = store.apply_delta(&delta).await.expect("seed failed");
    assert!(report.errors.is_empty(), "errors: {:?}", report.errors);
}

#[tokio::test]
async fn analytics_blast_radius_walks_dependency_edges() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");
    seed_dependency_graph(&store).await;

    let report = store
        .run_analytics(&AnalyticsSpec {
            mode: AnalyticsMode::BlastRadius,
            id: Some(CanonicalId::from("an-db")),
            depth: 3,
            limit: 50,
        })
        .await
        .expect("blast_radius failed");
    assert_eq!(report.mode, AnalyticsMode::BlastRadius);
    let rows: Vec<(&str, Option<u8>)> = report
        .rows
        .iter()
        .map(|r| (r.id.as_str(), r.distance))
        .collect();
    assert_eq!(
        rows,
        vec![("an-app", Some(1)), ("an-worker", Some(2))],
        "engine used: {:?}, note: {:?}",
        report.engine,
        report.note
    );
    assert!(matches!(
        report.engine,
        AnalyticsEngine::Cypher | AnalyticsEngine::Apoc
    ));

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn analytics_critical_resources_ranks_resources() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");
    seed_dependency_graph(&store).await;

    let report = store
        .run_analytics(&AnalyticsSpec {
            mode: AnalyticsMode::CriticalResources,
            id: None,
            depth: 0,
            limit: 50,
        })
        .await
        .expect("critical_resources failed");
    assert_eq!(report.mode, AnalyticsMode::CriticalResources);
    let ids: Vec<&str> = report.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["an-db", "an-bucket"],
        "db has two inbound edges, bucket one; engine: {:?}, note: {:?}",
        report.engine,
        report.note
    );
    assert!(report.rows.iter().all(|r| r.kind == NodeKind::Resource));
    match report.engine {
        AnalyticsEngine::Gds => {
            assert!(!report.degraded, "gds run is not degraded");
            assert!(report.note.is_none());
            assert!(
                report.rows[0].score > report.rows[1].score && report.rows[1].score > 0.0,
                "pagerank scores are positive and ordered, got {:?}",
                report.rows.iter().map(|r| r.score).collect::<Vec<_>>()
            );
        }
        AnalyticsEngine::Cypher => {
            assert!(report.degraded, "degree fallback marks the report degraded");
            assert!(report.note.is_some());
            assert_eq!(report.rows[0].score, 2.0);
            assert_eq!(report.rows[1].score, 1.0);
        }
        other => panic!("unexpected engine {other:?}"),
    }

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn analytics_bridges_and_clusters_degrade_gracefully() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");
    seed_dependency_graph(&store).await;

    for mode in [AnalyticsMode::Bridges, AnalyticsMode::Clusters] {
        let report = store
            .run_analytics(&AnalyticsSpec {
                mode,
                id: None,
                depth: 0,
                limit: 50,
            })
            .await
            .expect("analytics failed");
        assert_eq!(report.mode, mode);
        if report.degraded {
            assert!(report.rows.is_empty(), "skipped analysis returns no rows");
            assert!(report.note.is_some(), "degradation must carry a note");
        } else {
            assert_eq!(report.engine, AnalyticsEngine::Gds);
            assert!(report.note.is_none());
            assert_eq!(
                report.rows.len(),
                4,
                "seed has four nodes, all projected into the graph"
            );
            assert!(report.rows.iter().any(|r| r.kind == NodeKind::Function));
            assert!(report.rows.iter().any(|r| r.kind == NodeKind::Resource));
            if mode == AnalyticsMode::Clusters {
                assert!(
                    report.rows.iter().all(|r| r.component.is_some()),
                    "wcc assigns every node a component"
                );
            }
        }
    }

    store.reset().await.expect("reset failed");
}

#[tokio::test]
async fn analytics_blast_radius_requires_id() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    let err = store
        .run_analytics(&AnalyticsSpec {
            mode: AnalyticsMode::BlastRadius,
            id: None,
            depth: 2,
            limit: 10,
        })
        .await
        .expect_err("missing id must fail");
    assert!(err.to_string().contains("requires a node id"));
}
