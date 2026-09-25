use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use ckg_domain::{CanonicalId, NodeKind, RelationKind};
use ckg_git::RepoHandle;
use ckg_graph_api::{GraphApi, GraphStore, MockStore};
use ckg_graph_delta::{DeltaOp, GraphDelta};
use ckg_indexer::jobs::run_index_pass;
use ckg_indexer::pipeline::{commit_id, index_branch, load_prev_maps};
use ckg_indexer::state::{IndexState, state_path};
use ckg_repository_config::{RepositoryConfig, WorkspaceConfig};

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

fn init_two_branch_repo(dir: &Path) -> (String, String) {
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    write(&dir.join("src/lib.rs"), "fn foo() {}\n");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "main v1"]);
    let sha_main = git(dir, &["rev-parse", "HEAD"]);

    git(dir, &["checkout", "-q", "-b", "develop"]);
    write(&dir.join("src/lib.rs"), "fn foo() {}\n\nfn bar() {}\n");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "develop v1"]);
    let sha_dev = git(dir, &["rev-parse", "HEAD"]);

    (sha_main, sha_dev)
}

fn repo_config() -> RepositoryConfig {
    RepositoryConfig::new("local", "org/local", "https://example.com/local.git")
        .with_branches(["main", "develop"])
}

fn two_branch_workspace(repo: &RepositoryConfig) -> WorkspaceConfig {
    WorkspaceConfig {
        repositories: vec![repo.clone()],
        default_branches: vec![],
        ..Default::default()
    }
}

async fn contains_state_targets(store: &dyn GraphStore, repo: &str, sha: &str) -> Vec<CanonicalId> {
    let commit = commit_id(repo, sha);
    let mut edges = store
        .get_edges_from(&commit, Some(RelationKind::ContainsStateOf), 100)
        .await
        .expect("contains edges");
    edges.sort_by(|a, b| a.to.as_str().cmp(b.to.as_str()));
    edges.into_iter().map(|e| e.to).collect()
}

async fn find_by_name(store: &dyn GraphStore, name: &str) -> Option<CanonicalId> {
    let nodes = store.find_nodes_by_name(name, 10).await.expect("find");
    nodes.into_iter().find(|n| n.name == name).map(|n| n.id)
}

#[tokio::test]
async fn multi_branch_index_creates_branch_scaffolding_and_contains_state() {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let (sha_main, sha_dev) = init_two_branch_repo(repo_dir.path());
    assert_ne!(sha_main, sha_dev);

    let handle = RepoHandle::open(repo_dir.path()).expect("open repo");
    let repo = repo_config();
    let workspace = two_branch_workspace(&repo);
    let store = Arc::new(MockStore::new());
    let mut state = IndexState::default();

    for branch in workspace.resolved_branches(&repo) {
        index_branch(
            &handle,
            &*store,
            cache_dir.path(),
            &repo,
            &branch,
            None,
            None,
            false,
            &mut state,
        )
        .await
        .unwrap_or_else(|err| panic!("index {branch} failed: {err:#}"));
    }

    assert_eq!(
        state.last_sha("local", "main").map(str::to_string),
        Some(sha_main.clone())
    );
    assert_eq!(
        state.last_sha("local", "develop").map(str::to_string),
        Some(sha_dev.clone())
    );

    for branch in ["main", "develop"] {
        let nodes = store
            .find_nodes_by_name(branch, 10)
            .await
            .expect("find branch");
        assert!(
            nodes.iter().any(|n| n.kind == NodeKind::Branch),
            "missing Branch node for {branch}"
        );
    }

    let main_state = contains_state_targets(&*store, "org/local", &sha_main).await;
    let dev_state = contains_state_targets(&*store, "org/local", &sha_dev).await;
    assert!(!main_state.is_empty(), "main commit must CONTAINS_STATE_OF");
    assert!(
        !dev_state.is_empty(),
        "develop commit must CONTAINS_STATE_OF"
    );

    let foo_id = find_by_name(&*store, "foo").await.expect("foo node");
    let bar_id = find_by_name(&*store, "bar").await.expect("bar node");
    assert!(main_state.contains(&foo_id), "foo on main");
    assert!(dev_state.contains(&foo_id), "foo on develop");
    assert!(!main_state.contains(&bar_id), "bar is develop-only");
    assert!(dev_state.contains(&bar_id), "bar on develop");

    let repo_id = find_by_name(&*store, "org/local").await.expect("repo node");
    let branches = store
        .get_edges_from(&repo_id, Some(RelationKind::HasBranch), 10)
        .await
        .expect("has branch edges");
    assert_eq!(branches.len(), 2, "repo must point at main and develop");

    let api = GraphApi::with_defaults(store.clone());
    let on_main = api
        .find_symbol("bar", None, 10, Some("main"))
        .await
        .expect("find bar on main");
    assert!(on_main.data.is_empty(), "bar is not on main");
    let on_dev = api
        .find_symbol("bar", None, 10, Some("develop"))
        .await
        .expect("find bar on develop");
    assert_eq!(on_dev.data.len(), 1);
    let on_main = api
        .find_symbol("foo", None, 10, Some("main"))
        .await
        .expect("find foo on main");
    assert_eq!(on_main.data.len(), 1, "foo shared by both branches");
}

#[tokio::test]
async fn reindex_develop_preserves_main_contains_state_of() {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let (sha_main, _sha_dev) = init_two_branch_repo(repo_dir.path());

    let handle = RepoHandle::open(repo_dir.path()).expect("open repo");
    let repo = repo_config();
    let workspace = two_branch_workspace(&repo);
    let store = MockStore::new();
    let mut state = IndexState::default();

    for branch in workspace.resolved_branches(&repo) {
        index_branch(
            &handle,
            &store,
            cache_dir.path(),
            &repo,
            &branch,
            None,
            None,
            false,
            &mut state,
        )
        .await
        .unwrap_or_else(|err| panic!("index {branch} failed: {err:#}"));
    }

    let foo_id = find_by_name(&store, "foo").await.expect("foo node");
    let bar_id = find_by_name(&store, "bar").await.expect("bar node");
    let main_state_before = contains_state_targets(&store, "org/local", &sha_main).await;
    assert!(main_state_before.contains(&foo_id));
    assert!(!main_state_before.contains(&bar_id));

    write(
        &repo_dir.path().join("src/lib.rs"),
        "fn foo() {}\n\nfn baz() {}\n",
    );
    git(repo_dir.path(), &["add", "-A"]);
    git(repo_dir.path(), &["commit", "-q", "-m", "develop v2"]);
    let sha_dev2 = git(repo_dir.path(), &["rev-parse", "HEAD"]);

    let last_sha = state.last_sha("local", "develop").map(str::to_string);
    let prev_maps = load_prev_maps(cache_dir.path(), "local", "develop")
        .expect("load prev")
        .expect("prev maps present");
    let report = index_branch(
        &handle,
        &store,
        cache_dir.path(),
        &repo,
        "develop",
        last_sha.as_deref(),
        Some(prev_maps),
        false,
        &mut state,
    )
    .await
    .expect("reindex develop");
    assert!(!report.up_to_date);
    assert_eq!(
        state.last_sha("local", "develop").map(str::to_string),
        Some(sha_dev2.clone())
    );

    let main_state_after = contains_state_targets(&store, "org/local", &sha_main).await;
    assert_eq!(
        main_state_before, main_state_after,
        "develop re-index must not touch main's CONTAINS_STATE_OF"
    );

    let dev_state = contains_state_targets(&store, "org/local", &sha_dev2).await;
    assert!(dev_state.contains(&foo_id), "develop still contains foo");
    let baz_id = find_by_name(&store, "baz").await.expect("baz node");
    assert!(dev_state.contains(&baz_id), "develop gained baz");
    assert!(!dev_state.contains(&bar_id), "develop dropped bar");
    assert!(
        find_by_name(&store, "bar").await.is_none(),
        "bar removed from the store"
    );
}

#[tokio::test]
async fn default_branches_drive_index_pass_when_monitored_empty() {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let config_dir = tempfile::tempdir().expect("config tempdir");
    let (sha_main, sha_dev) = init_two_branch_repo(repo_dir.path());

    let workspace = WorkspaceConfig {
        repositories: vec![
            RepositoryConfig::new("local", "org/local", "https://example.com/local.git")
                .with_branches(Vec::<String>::new())
                .with_local_path(repo_dir.path().to_string_lossy().as_ref()),
        ],
        default_branches: vec!["main".to_string(), "develop".to_string()],
        ..Default::default()
    };
    let config_path = config_dir.path().join("workspace.toml");
    workspace.save(&config_path).expect("save config");

    let store = MockStore::new();
    let outcome = run_index_pass(&store, &config_path, cache_dir.path(), None, None, false)
        .await
        .expect("index pass");
    assert!(
        outcome.failures.is_empty(),
        "failures: {:?}",
        outcome.failures
    );
    assert_eq!(outcome.indexed, 2, "both default branches indexed");

    let state = IndexState::load(&state_path(cache_dir.path())).expect("load state");
    assert_eq!(
        state.last_sha("local", "main").map(str::to_string),
        Some(sha_main.clone())
    );
    assert_eq!(
        state.last_sha("local", "develop").map(str::to_string),
        Some(sha_dev.clone())
    );

    for branch in ["main", "develop"] {
        let nodes = store.find_nodes_by_name(branch, 10).await.expect("find");
        assert!(
            nodes.iter().any(|n| n.kind == NodeKind::Branch),
            "missing Branch node for {branch}"
        );
    }

    let main_state = contains_state_targets(&store, "org/local", &sha_main).await;
    assert!(
        !main_state.is_empty(),
        "default branch main indexed into store"
    );
    let dev_state = contains_state_targets(&store, "org/local", &sha_dev).await;
    assert!(!dev_state.is_empty());
}

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn connect_or_skip() -> Option<ckg_neo4j_store::Neo4jStore> {
    use ckg_neo4j_store::Neo4jStore;
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

#[tokio::test]
async fn multi_branch_index_on_neo4j_guards_shared_symbol_delete() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let (sha_main, sha_dev) = init_two_branch_repo(repo_dir.path());

    let handle = RepoHandle::open(repo_dir.path()).expect("open repo");
    let repo = repo_config();
    let workspace = two_branch_workspace(&repo);
    let mut state = IndexState::default();

    for branch in workspace.resolved_branches(&repo) {
        index_branch(
            &handle,
            &store,
            cache_dir.path(),
            &repo,
            &branch,
            None,
            None,
            false,
            &mut state,
        )
        .await
        .unwrap_or_else(|err| panic!("index {branch}: {err:#}"));
    }

    let foo_id = find_by_name(&store, "foo").await.expect("foo node");
    let bar_id = find_by_name(&store, "bar").await.expect("bar node");
    let dev_commit = commit_id("org/local", &sha_dev);
    let main_commit = commit_id("org/local", &sha_main);

    let main_edges = store
        .get_edges_from(&main_commit, Some(RelationKind::ContainsStateOf), 100)
        .await
        .expect("main edges");
    assert!(
        main_edges.iter().any(|e| e.to == foo_id),
        "main contains foo"
    );

    let partial = GraphDelta {
        ops: vec![
            DeltaOp::DeleteRelationship {
                kind: RelationKind::ContainsStateOf,
                from: dev_commit.clone(),
                to: foo_id.clone(),
            },
            DeltaOp::DeleteNode(foo_id.clone()),
        ],
        base_revision: None,
        target_revision: None,
    };
    let report = store.apply_delta(&partial).await.expect("partial delete");
    assert_eq!(report.deleted_nodes, 0, "shared foo must not be deleted");
    assert_eq!(report.nodes_retained, 1, "foo retained via main");
    assert!(
        store.get_node(&foo_id).await.expect("get").is_some(),
        "foo still present after develop-side delete"
    );
    let main_edges = store
        .get_edges_from(&main_commit, Some(RelationKind::ContainsStateOf), 100)
        .await
        .expect("main edges");
    assert!(
        main_edges.iter().any(|e| e.to == foo_id),
        "main edge intact"
    );

    let exclusive = GraphDelta {
        ops: vec![
            DeltaOp::DeleteRelationship {
                kind: RelationKind::ContainsStateOf,
                from: dev_commit,
                to: bar_id.clone(),
            },
            DeltaOp::DeleteNode(bar_id.clone()),
        ],
        base_revision: None,
        target_revision: None,
    };
    let report = store.apply_delta(&exclusive).await.expect("delete bar");
    assert_eq!(report.deleted_nodes, 1, "bar only lived on develop");
    assert_eq!(report.nodes_retained, 0);
    assert!(store.get_node(&bar_id).await.expect("get").is_none());

    store.reset().await.expect("reset failed");
}
