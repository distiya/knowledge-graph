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

fn commit_id(repo: &str, sha: &str) -> CanonicalId {
    CanonicalId::from(format!("commit::{repo}::{sha}"))
}

fn symbol(repo: &str, id: &str, name: &str, sha: &str) -> GraphNode {
    GraphNode::new(NodeKind::Function, CanonicalId::from(id), name)
        .with_location(SourceLocation::new(repo, sha, "src/lib.rs", 1, 0, 2, 0))
}

async fn seed_multi_branch(store: &Neo4jStore) {
    let repo = "org/multi";
    let repo_id = CanonicalId::from("repo-org-multi");
    let sha_main = "sha-main";
    let sha_dev = "sha-dev";
    let main_commit = commit_id(repo, sha_main);
    let dev_commit = commit_id(repo, sha_dev);

    let delta = GraphDelta {
        ops: vec![
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Repository, repo_id.clone(), repo)
                    .with_property("repository", serde_json::json!(repo)),
            ),
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
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, main_commit.clone(), sha_main)
                    .with_property("repository", serde_json::json!(repo)),
            ),
            DeltaOp::CreateNode(
                GraphNode::new(NodeKind::Commit, dev_commit.clone(), sha_dev)
                    .with_property("repository", serde_json::json!(repo)),
            ),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::HasBranch,
                repo_id.clone(),
                CanonicalId::from("br-main"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::HasBranch,
                repo_id,
                CanonicalId::from("br-dev"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-main"),
                main_commit.clone(),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::PointsTo,
                CanonicalId::from("br-dev"),
                dev_commit.clone(),
            )),
            DeltaOp::CreateNode(symbol(repo, "sym-alpha", "alpha", sha_main)),
            DeltaOp::CreateNode(symbol(repo, "sym-beta", "beta", sha_dev)),
            DeltaOp::CreateNode(symbol(repo, "sym-gamma", "gamma", sha_main)),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                main_commit.clone(),
                CanonicalId::from("sym-alpha"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                dev_commit.clone(),
                CanonicalId::from("sym-beta"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                main_commit,
                CanonicalId::from("sym-gamma"),
            )),
            DeltaOp::CreateRelationship(GraphEdge::new(
                RelationKind::ContainsStateOf,
                dev_commit,
                CanonicalId::from("sym-gamma"),
            )),
        ],
        base_revision: None,
        target_revision: None,
    };
    let report = store.apply_delta(&delta).await.expect("seed failed");
    assert!(report.is_success(), "seed errors: {:?}", report.errors);
}

#[tokio::test]
async fn live_branch_scoped_find_compare_and_guarded_delete() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");
    seed_multi_branch(&store).await;

    let store = Arc::new(store);
    let api = GraphApi::with_defaults(store.clone());
    let repo = "org/multi";

    let alpha_on_main = api
        .find_symbol("alpha", None, 10, Some("main"))
        .await
        .expect("find alpha on main");
    assert_eq!(alpha_on_main.data.len(), 1);
    assert_eq!(alpha_on_main.data[0].id.as_str(), "sym-alpha");

    let alpha_on_dev = api
        .find_symbol("alpha", None, 10, Some("develop"))
        .await
        .expect("find alpha on develop");
    assert!(
        alpha_on_dev.data.is_empty(),
        "alpha is only contained by main's commit"
    );

    let beta_on_dev = api
        .find_symbol("beta", None, 10, Some("develop"))
        .await
        .expect("find beta on develop");
    assert_eq!(beta_on_dev.data.len(), 1);

    let comparison = api
        .compare_branch_state(repo, "main", "develop")
        .await
        .expect("compare");
    assert!(comparison.data.branch_a_indexed);
    assert!(comparison.data.branch_b_indexed);
    assert_eq!(comparison.data.common, vec!["gamma".to_string()]);
    assert_eq!(comparison.data.only_in_a, vec!["alpha".to_string()]);
    assert_eq!(comparison.data.only_in_b, vec!["beta".to_string()]);

    let gamma_id = CanonicalId::from("sym-gamma");
    let partial = GraphDelta {
        ops: vec![
            DeltaOp::DeleteRelationship {
                kind: RelationKind::ContainsStateOf,
                from: commit_id(repo, "sha-main"),
                to: gamma_id.clone(),
            },
            DeltaOp::DeleteNode(gamma_id.clone()),
        ],
        base_revision: None,
        target_revision: None,
    };
    let report = store.apply_delta(&partial).await.expect("partial delete");
    assert_eq!(report.deleted_nodes, 0);
    assert_eq!(report.nodes_retained, 1, "gamma retained via develop");
    assert!(
        store.get_node(&gamma_id).await.expect("get").is_some(),
        "shared symbol must survive a single-branch delete"
    );

    let gamma_on_main = api
        .find_symbol("gamma", None, 10, Some("main"))
        .await
        .expect("find gamma on main");
    assert!(gamma_on_main.data.is_empty(), "main's edge was removed");
    let gamma_on_dev = api
        .find_symbol("gamma", None, 10, Some("develop"))
        .await
        .expect("find gamma on develop");
    assert_eq!(gamma_on_dev.data.len(), 1, "develop still contains gamma");

    let beta_id = CanonicalId::from("sym-beta");
    let final_delete = GraphDelta {
        ops: vec![
            DeltaOp::DeleteRelationship {
                kind: RelationKind::ContainsStateOf,
                from: commit_id(repo, "sha-dev"),
                to: beta_id.clone(),
            },
            DeltaOp::DeleteNode(beta_id.clone()),
        ],
        base_revision: None,
        target_revision: None,
    };
    let report = store.apply_delta(&final_delete).await.expect("delete beta");
    assert_eq!(report.deleted_nodes, 1, "beta only lived on develop");
    assert!(store.get_node(&beta_id).await.expect("get").is_none());

    store.reset().await.expect("reset failed");
}
