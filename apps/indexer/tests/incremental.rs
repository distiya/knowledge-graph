use std::path::Path;
use std::process::Command;

use ckg_git::RepoHandle;
use ckg_graph_api::{GraphStore, MockStore};
use ckg_indexer::pipeline::{
    incremental_plan, index_branch, load_prev_maps, merge_incremental, should_index,
};
use ckg_indexer::state::IndexState;
use ckg_repository_config::RepositoryConfig;

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

fn init_repo_with_two_files(dir: &Path) -> String {
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "fn alpha() {}\n\nfn beta() {\n    alpha();\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/util.rs"), "fn helper() -> u8 {\n    1\n}\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "v1"]);
    git(dir, &["rev-parse", "HEAD"])
}

fn commit_lib_change(dir: &Path) -> String {
    std::fs::write(
        dir.join("src/lib.rs"),
        "fn alpha() {\n    let _x = 1;\n}\n\nfn beta2() {\n    alpha();\n}\n",
    )
    .unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "v2"]);
    git(dir, &["rev-parse", "HEAD"])
}

fn repo_config() -> RepositoryConfig {
    RepositoryConfig::new("local", "org/local", "https://example.com/local.git")
        .with_branches(["main"])
}

#[tokio::test]
async fn second_index_analyzes_only_changed_file() {
    let repo_dir = tempfile::tempdir().expect("repo tempdir");
    let cache_dir = tempfile::tempdir().expect("cache tempdir");
    let sha1 = init_repo_with_two_files(repo_dir.path());

    let handle = RepoHandle::open(repo_dir.path()).expect("open repo");
    let repo = repo_config();
    let store = MockStore::new();
    let mut state = IndexState::default();

    let first = index_branch(
        &handle,
        &store,
        cache_dir.path(),
        &repo,
        "main",
        None,
        None,
        false,
        &mut state,
    )
    .await
    .expect("first index");
    assert!(!first.up_to_date);
    assert_eq!(first.files_analyzed, 2, "first index analyzes both files");
    assert_eq!(state.last_sha("local", "main"), Some(sha1.as_str()));

    let sha2 = commit_lib_change(repo_dir.path());
    assert_ne!(sha1, sha2);

    let last_sha = state.last_sha("local", "main").map(str::to_string);
    let prev_maps = load_prev_maps(cache_dir.path(), "local", "main")
        .expect("load prev")
        .expect("prev maps present");

    let util_nodes_before: Vec<_> = prev_maps
        .0
        .values()
        .filter(|node| {
            node.location
                .as_ref()
                .is_some_and(|loc| loc.path == "src/util.rs")
        })
        .cloned()
        .collect();
    assert!(
        !util_nodes_before.is_empty(),
        "expected symbols from src/util.rs in prev maps"
    );

    let second = index_branch(
        &handle,
        &store,
        cache_dir.path(),
        &repo,
        "main",
        last_sha.as_deref(),
        Some(prev_maps),
        false,
        &mut state,
    )
    .await
    .expect("second index");

    assert!(!second.up_to_date);
    assert_eq!(
        second.files_analyzed, 1,
        "incremental index must analyze only the changed file"
    );
    assert!(second.delta_ops > 0);
    assert_eq!(state.last_sha("local", "main"), Some(sha2.as_str()));

    let (new_nodes, _) = load_prev_maps(cache_dir.path(), "local", "main")
        .expect("load new")
        .expect("new maps present");
    let util_nodes_after: Vec<_> = new_nodes
        .values()
        .filter(|node| {
            node.location
                .as_ref()
                .is_some_and(|loc| loc.path == "src/util.rs")
        })
        .cloned()
        .collect();
    assert_eq!(
        util_nodes_before, util_nodes_after,
        "unchanged file nodes must be byte-identical after incremental index"
    );

    let lib_names: Vec<&str> = new_nodes
        .values()
        .filter(|node| {
            node.location
                .as_ref()
                .is_some_and(|loc| loc.path == "src/lib.rs")
        })
        .map(|node| node.name.as_str())
        .collect();
    assert!(lib_names.contains(&"beta2"));
    assert!(!lib_names.contains(&"beta"));

    let beta2 = store
        .find_nodes_by_name("beta2", 10)
        .await
        .expect("find beta2");
    assert!(
        beta2.iter().any(|node| node.name == "beta2"),
        "renamed function present after incremental index"
    );

    let beta = store
        .find_nodes_by_name("beta", 10)
        .await
        .expect("find beta");
    assert!(
        !beta.iter().any(|node| node.name == "beta"),
        "old function name removed after incremental index"
    );

    let helper = store
        .find_nodes_by_name("helper", 10)
        .await
        .expect("find helper");
    assert!(
        helper.iter().any(|node| node.name == "helper"),
        "unchanged file symbols survive incremental index"
    );

    let third = index_branch(
        &handle,
        &store,
        cache_dir.path(),
        &repo,
        "main",
        state
            .last_sha("local", "main")
            .map(str::to_string)
            .as_deref(),
        load_prev_maps(cache_dir.path(), "local", "main").unwrap(),
        false,
        &mut state,
    )
    .await
    .expect("third index");
    assert!(third.up_to_date, "unchanged sha is skipped");
    assert_eq!(should_index(Some(&sha2), &sha2, false), false);
}

#[test]
fn incremental_plan_covers_add_modify_delete_rename() {
    use ckg_git::FileDiff;
    let diff = FileDiff {
        added: vec!["src/new.rs".into()],
        modified: vec!["src/lib.rs".into()],
        deleted: vec!["src/gone.rs".into()],
        renamed: vec![("src/old.rs".into(), "src/moved.rs".into())],
    };
    let plan = incremental_plan(&diff);
    assert_eq!(
        plan.analyze,
        vec![
            "src/lib.rs".to_string(),
            "src/moved.rs".to_string(),
            "src/new.rs".to_string()
        ]
    );
    assert!(plan.remove_paths.contains("src/gone.rs"));
    assert!(plan.remove_paths.contains("src/old.rs"));
    assert!(plan.remove_paths.contains("src/lib.rs"));
    assert!(plan.remove_paths.contains("src/moved.rs"));
}

#[test]
fn merge_incremental_strips_changed_paths_and_old_commits() {
    use ckg_domain::{CanonicalId, GraphNode, NodeKind, SourceLocation};
    use ckg_graph_delta::{EdgeMap, NodeMap};

    let mut prev_nodes = NodeMap::new();
    let keep = GraphNode::new(NodeKind::Function, CanonicalId::from("keep"), "keep").with_location(
        SourceLocation::new("org/r", "s1", "src/keep.rs", 1, 0, 2, 0),
    );
    let change =
        GraphNode::new(NodeKind::Function, CanonicalId::from("change"), "old").with_location(
            SourceLocation::new("org/r", "s1", "src/change.rs", 1, 0, 2, 0),
        );
    let commit = GraphNode::new(NodeKind::Commit, CanonicalId::from("c1"), "s1");
    prev_nodes.insert("keep".into(), keep);
    prev_nodes.insert("change".into(), change);
    prev_nodes.insert("c1".into(), commit);

    let prev_edges = EdgeMap::new();
    let fresh_nodes: NodeMap = [(
        "change".to_string(),
        GraphNode::new(NodeKind::Function, CanonicalId::from("change"), "new").with_location(
            SourceLocation::new("org/r", "s2", "src/change.rs", 1, 0, 3, 0),
        ),
    )]
    .into_iter()
    .collect();

    let remove: std::collections::BTreeSet<String> =
        ["src/change.rs".to_string()].into_iter().collect();
    let (nodes, _edges) = merge_incremental(
        &(prev_nodes.clone(), prev_edges.clone()),
        &fresh_nodes,
        &EdgeMap::new(),
        &remove,
        "org/r",
        "main",
        "s2",
    );

    assert!(nodes.contains_key("keep"), "unchanged symbol kept");
    assert_eq!(
        nodes.get("change").map(|n| n.name.as_str()),
        Some("new"),
        "changed file replaced with fresh analysis"
    );
    assert!(
        !nodes.contains_key("c1"),
        "old commit node dropped; scaffolding re-adds current sha"
    );
    assert!(
        nodes.values().any(|n| n.kind == NodeKind::Commit),
        "new commit scaffolding present"
    );
    assert!(
        nodes.values().any(|n| n.kind == NodeKind::Repository),
        "repository scaffolding present"
    );
}
