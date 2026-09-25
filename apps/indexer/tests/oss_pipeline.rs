use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use ckg_domain::{Analyzer, CanonicalId, NodeKind, ProvenanceSource, RelationKind};
use ckg_git::{RepoHandle, detect_language};
use ckg_graph_api::{GraphApi, GraphStore, MockStore};
use ckg_graph_delta::{EdgeMap, NodeMap, apply_ops_to_maps, diff_graphs};
use ckg_indexer::pipeline::{analyze_files, maps_delta, normalized_to_maps};
use ckg_mcp_server::McpServer;
use ckg_normalizer::{NormalizedGraph, Normalizer};
use ckg_tree_sitter_analyzer::TreeSitterAnalyzer;

const AHO_CORASICK_URL: &str = "https://github.com/BurntSushi/aho-corasick";
const MEMCHR_URL: &str = "https://github.com/BurntSushi/memchr";

fn oss_cache() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/oss-cache")
        .canonicalize()
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/oss-cache")
        })
}

fn ensure_clone(url: &str, dir_name: &str) -> Option<PathBuf> {
    let root = oss_cache();
    let dir = root.join(dir_name);
    if dir.join(".git").exists() {
        return Some(dir);
    }
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    if let Err(err) = std::fs::create_dir_all(&root) {
        eprintln!("skipping: cannot create {}: {err}", root.display());
        return None;
    }
    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(&dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output();
    match output {
        Ok(out) if out.status.success() => Some(dir),
        Ok(out) => {
            eprintln!(
                "skipping: clone of {url} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            None
        }
        Err(err) => {
            eprintln!("skipping: cannot run git to clone {url}: {err}");
            None
        }
    }
}

fn open_head(path: &Path) -> Option<(RepoHandle, String)> {
    let handle = match RepoHandle::open(path) {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("skipping: cannot open {}: {err}", path.display());
            return None;
        }
    };
    match handle.head_commit_sha() {
        Ok(sha) => Some((handle, sha)),
        Err(err) => {
            eprintln!("skipping: cannot resolve HEAD of {}: {err}", path.display());
            None
        }
    }
}

fn normalize_repo(path: &Path, repository: &str) -> Option<(NormalizedGraph, NodeMap, EdgeMap)> {
    let (handle, sha) = open_head(path)?;
    let analysis = match analyze_files(&handle, &sha, repository, &[]) {
        Ok(analysis) => analysis,
        Err(err) => {
            eprintln!("skipping: analysis failed for {repository}: {err:#}");
            return None;
        }
    };
    let graph = Normalizer::normalize(analysis.outputs);
    let (nodes, edges, _) = normalized_to_maps(&graph);
    Some((graph, nodes, edges))
}

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

#[test]
fn pipeline_extracts_symbols_from_aho_corasick() {
    let Some(dir) = ensure_clone(AHO_CORASICK_URL, "aho-corasick") else {
        return;
    };
    let Some((graph, _, _)) = normalize_repo(&dir, "BurntSushi/aho-corasick") else {
        return;
    };

    let function_nodes = graph
        .nodes
        .values()
        .filter(|n| matches!(n.kind, NodeKind::Function | NodeKind::Method))
        .count();
    assert!(
        function_nodes > 20,
        "expected > 20 function nodes, got {function_nodes}"
    );

    let calls = graph
        .edges
        .values()
        .filter(|e| e.kind == RelationKind::Calls)
        .count();
    assert!(calls > 0, "expected CALLS edges");

    assert!(!graph.nodes.is_empty());
    for node in graph.nodes.values() {
        assert!(
            node.provenance
                .iter()
                .any(|p| p.source == ProvenanceSource::TreeSitter),
            "node {} lacks tree-sitter provenance",
            node.name
        );
        let hash = node
            .content_hash
            .as_ref()
            .unwrap_or_else(|| panic!("node {} lacks content_hash", node.name));
        assert!(!hash.is_empty());
        let location = node
            .location
            .as_ref()
            .unwrap_or_else(|| panic!("node {} lacks location", node.name));
        assert!(location.start_line >= 1);
        assert!(location.end_line >= location.start_line);
        assert!(location.start_column >= 1);
        assert!(location.end_column >= 1);
    }
}

#[test]
fn canonical_ids_stable_across_runs() {
    let Some(dir) = ensure_clone(AHO_CORASICK_URL, "aho-corasick") else {
        return;
    };
    let Some((handle, sha)) = open_head(&dir) else {
        return;
    };
    let path = "src/ahocorasick.rs";
    let content = handle
        .file_at(&sha, path)
        .ok()
        .flatten()
        .expect("src/ahocorasick.rs must exist");
    let language = detect_language(path).expect("rust file");

    let analyzer = TreeSitterAnalyzer::new();
    let run = |analyzer: &TreeSitterAnalyzer| -> Vec<String> {
        analyzer
            .analyze("BurntSushi/aho-corasick", &sha, path, &content, language)
            .symbols
            .into_iter()
            .map(|s| s.canonical_id.to_string())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    };

    let first = run(&analyzer);
    let second = run(&analyzer);
    assert!(!first.is_empty(), "expected symbols from ahocorasick.rs");
    assert_eq!(first, second);
}

#[test]
fn diff_graphs_incremental_between_commits() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-b", "main"]);

    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "fn alpha() {}\n\nfn beta() {\n    alpha();\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/gone.rs"), "fn gone() {}\n").unwrap();
    let sha1 = {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "v1"]);
        git(dir, &["rev-parse", "HEAD"])
    };

    std::fs::write(
        dir.join("src/lib.rs"),
        "fn alpha() {\n    let _x = 1;\n}\n\nfn beta2() {\n    alpha();\n}\n",
    )
    .unwrap();
    std::fs::remove_file(dir.join("src/gone.rs")).unwrap();
    std::fs::write(dir.join("src/extra.rs"), "fn gamma() {}\n").unwrap();
    let sha2 = {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "v2"]);
        git(dir, &["rev-parse", "HEAD"])
    };

    let repo = RepoHandle::open(dir).expect("open repo");
    let diff = repo.diff_files(&sha1, &sha2).expect("diff");
    assert_eq!(diff.modified, vec!["src/lib.rs".to_string()]);
    assert_eq!(diff.added, vec!["src/extra.rs".to_string()]);
    assert_eq!(diff.deleted, vec!["src/gone.rs".to_string()]);

    let analyzer = TreeSitterAnalyzer::new();
    let analyze_at = |sha: &str| -> NormalizedGraph {
        let files = repo.list_files_at(sha).expect("list files");
        let outputs = files
            .iter()
            .filter(|path| detect_language(path) == Some("rust"))
            .filter_map(|path| {
                repo.file_at(sha, path)
                    .ok()
                    .flatten()
                    .map(|content| (path.clone(), content))
            })
            .map(|(path, content)| analyzer.analyze_file("org/local", sha, &path, &content, "rust"))
            .collect();
        Normalizer::normalize(outputs)
    };

    let graph1 = analyze_at(&sha1);
    let graph2 = analyze_at(&sha2);
    let (nodes1, edges1, _) = normalized_to_maps(&graph1);
    let (nodes2, edges2, _) = normalized_to_maps(&graph2);

    let names =
        |nodes: &NodeMap| -> BTreeSet<String> { nodes.values().map(|n| n.name.clone()).collect() };
    assert!(names(&nodes1).contains("beta"));
    assert!(names(&nodes2).contains("beta2"));
    assert!(names(&nodes2).contains("gamma"));
    assert!(!names(&nodes2).contains("gone"));

    let delta = diff_graphs(&nodes1, &nodes2, &edges1, &edges2);
    assert!(!delta.is_empty(), "incremental delta must be non-empty");
    assert!(
        delta
            .ops
            .iter()
            .any(|op| matches!(op, ckg_graph_delta::DeltaOp::CreateNode(n) if n.name == "gamma"))
    );
    assert!(delta.ops.iter().any(
        |op| matches!(op, ckg_graph_delta::DeltaOp::DeleteNode(id) if {
            nodes1.get(id.as_str()).map(|n| n.name == "gone").unwrap_or(false)
        })
    ));

    let mut applied_nodes = nodes1.clone();
    let mut applied_edges = edges1.clone();
    apply_ops_to_maps(&mut applied_nodes, &mut applied_edges, &delta);
    assert_eq!(applied_nodes, nodes2);
    assert_eq!(applied_edges, edges2);

    let mut applied_again_nodes = applied_nodes.clone();
    let mut applied_again_edges = applied_edges.clone();
    apply_ops_to_maps(&mut applied_again_nodes, &mut applied_again_edges, &delta);
    assert_eq!(applied_again_nodes, applied_nodes);
    assert_eq!(applied_again_edges, applied_edges);

    let rediff = diff_graphs(&applied_nodes, &nodes2, &applied_edges, &edges2);
    assert!(rediff.is_empty(), "re-diff after apply must be empty");
}

#[test]
fn cross_repo_two_repos() {
    let Some(aho_dir) = ensure_clone(AHO_CORASICK_URL, "aho-corasick") else {
        return;
    };
    let Some(memchr_dir) = ensure_clone(MEMCHR_URL, "memchr") else {
        return;
    };
    let Some((graph_a, _, _)) = normalize_repo(&aho_dir, "BurntSushi/aho-corasick") else {
        return;
    };
    let Some((graph_b, _, _)) = normalize_repo(&memchr_dir, "BurntSushi/memchr") else {
        return;
    };

    let (aho_handle, aho_sha) = open_head(&aho_dir).expect("aho head");
    let (memchr_handle, memchr_sha) = open_head(&memchr_dir).expect("memchr head");
    let analysis_a =
        analyze_files(&aho_handle, &aho_sha, "BurntSushi/aho-corasick", &[]).expect("analyze aho");
    let analysis_b = analyze_files(&memchr_handle, &memchr_sha, "BurntSushi/memchr", &[])
        .expect("analyze memchr");
    let combined = Normalizer::normalize(
        analysis_a
            .outputs
            .into_iter()
            .chain(analysis_b.outputs)
            .collect(),
    );

    assert_eq!(
        combined.nodes.len(),
        graph_a.nodes.len() + graph_b.nodes.len(),
        "combined node count must equal the sum of per-repo counts"
    );
    assert_eq!(
        combined.edges.len(),
        graph_a.edges.len() + graph_b.edges.len()
    );

    let ids_a: BTreeSet<String> = graph_a
        .nodes
        .keys()
        .map(|k| k.as_str().to_string())
        .collect();
    let ids_b: BTreeSet<String> = graph_b
        .nodes
        .keys()
        .map(|k| k.as_str().to_string())
        .collect();
    let overlap: Vec<&String> = ids_a.intersection(&ids_b).collect();
    assert!(
        overlap.is_empty(),
        "canonical ids must not collide across repos: {overlap:?}"
    );

    let languages: BTreeSet<&str> = combined
        .nodes
        .values()
        .filter_map(|n| n.language.as_deref())
        .collect();
    assert!(
        languages.contains("rust"),
        "expected rust symbols, got {languages:?}"
    );
    for node in combined.nodes.values() {
        let language = node
            .language
            .as_deref()
            .unwrap_or_else(|| panic!("node {} lacks language", node.name));
        assert!(
            ["rust", "toml", "markdown", "yaml", "json"].contains(&language),
            "unexpected language {language} on node {}",
            node.name
        );
    }
}

#[test]
fn open_repo_with_local_path_reads_commits_without_fetch() {
    use ckg_indexer::pipeline::open_repo;
    use ckg_repository_config::RepositoryConfig;

    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path().join("local");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    git(&dir, &["checkout", "-q", "-b", "main"]);
    std::fs::write(dir.join("lib.rs"), "fn local() {}\n").unwrap();
    let sha = {
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-q", "-m", "initial"]);
        git(&dir, &["rev-parse", "HEAD"])
    };

    let repo = RepositoryConfig::new("local", "", "")
        .with_local_path(dir.to_string_lossy().as_ref())
        .with_branches(["main"]);
    let cache = tmp.path().join("cache");
    std::fs::create_dir_all(&cache).unwrap();

    let handle = open_repo(&cache, &repo).expect("open local repo");
    assert_eq!(
        handle.head_commit_sha().unwrap(),
        sha,
        "commits must be visible without fetch"
    );
    assert!(
        handle.branch_commit_sha("main").unwrap().as_deref() == Some(sha.as_str()),
        "main branch resolvable from local path"
    );
    assert!(
        !cache.join("local").exists(),
        "local_path mode must not clone into the cache dir"
    );
}

#[tokio::test]
async fn multi_language_pipeline_smoke() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    git(dir, &["checkout", "-q", "-b", "main"]);

    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::create_dir_all(dir.join("java")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), "fn rust_main() {}\n").unwrap();
    std::fs::write(
        dir.join("scripts/sample.py"),
        "class Greeter:\n    def run(self):\n        pass\n\n\ndef py_helper():\n    pass\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("java/Example.java"),
        "package org.example;\n\npublic class Example {\n    public void greet() {}\n}\n",
    )
    .unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "multi language"]);
    let sha = git(dir, &["rev-parse", "HEAD"]);

    let handle = RepoHandle::open(dir).expect("open repo");
    let analysis =
        analyze_files(&handle, &sha, "org/multi-lang", &[]).expect("analyze multi language repo");
    assert_eq!(analysis.files_analyzed, 3, "one file per language");
    assert!(
        analysis.analyzer_errors.is_empty(),
        "analyzer errors: {:?}",
        analysis.analyzer_errors
    );

    let graph = Normalizer::normalize(analysis.outputs);
    assert!(
        graph.errors.is_empty(),
        "normalize errors: {:?}",
        graph.errors
    );

    let languages: BTreeSet<&str> = graph
        .nodes
        .values()
        .filter_map(|n| n.language.as_deref())
        .collect();
    for expected in ["rust", "python", "java"] {
        assert!(
            languages.contains(expected),
            "expected {expected} symbols, got {languages:?}"
        );
    }

    let names: BTreeSet<&str> = graph.nodes.values().map(|n| n.name.as_str()).collect();
    for expected in ["rust_main", "py_helper", "Greeter", "greet", "Example"] {
        assert!(names.contains(expected), "missing {expected} in {names:?}");
    }

    for node in graph.nodes.values() {
        assert_eq!(
            node.provenance
                .iter()
                .any(|p| p.source == ProvenanceSource::TreeSitter),
            true,
            "node {} lacks tree-sitter provenance",
            node.name
        );
    }

    let (nodes, edges, _) = normalized_to_maps(&graph);
    let store = std::sync::Arc::new(MockStore::new());
    let delta = maps_delta(None, &nodes, &edges, None, "multi".to_string());
    store.apply_delta(&delta).await.expect("seed mock");
    assert_eq!(store.node_count(), graph.nodes.len());

    for (name, language) in [
        ("rust_main", "rust"),
        ("py_helper", "python"),
        ("greet", "java"),
    ] {
        let node = graph
            .nodes
            .values()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("{name} node"));
        assert_eq!(
            node.language.as_deref(),
            Some(language),
            "{name} language mismatch"
        );
    }
}

async fn connect_or_skip() -> Option<ckg_neo4j_store::Neo4jStore> {
    use ckg_neo4j_store::{GraphStore, Neo4jStore};
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

static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test]
async fn neo4j_roundtrip_oss() {
    let _db = DB_LOCK.lock().await;
    let Some(store) = connect_or_skip().await else {
        return;
    };
    store.reset().await.expect("reset failed");

    let Some(dir) = ensure_clone(AHO_CORASICK_URL, "aho-corasick") else {
        return;
    };
    let Some((_, nodes, edges)) = normalize_repo(&dir, "BurntSushi/aho-corasick") else {
        return;
    };

    let delta = maps_delta(None, &nodes, &edges, None, "oss".to_string());
    let report = store.apply_delta(&delta).await.expect("apply failed");
    assert!(report.is_success(), "apply errors: {:?}", report.errors);
    assert_eq!(report.created_nodes, nodes.len() as u64);

    let known = store
        .find_nodes_by_name("AhoCorasick", 10)
        .await
        .expect("find");
    let target_name = if known.is_empty() {
        nodes
            .values()
            .find(|n| n.kind == NodeKind::Function)
            .map(|n| n.name.clone())
            .expect("graph must contain a function")
    } else {
        "AhoCorasick".to_string()
    };
    let found = store
        .find_nodes_by_name(&target_name, 10)
        .await
        .expect("find known symbol");
    assert!(!found.is_empty(), "expected {target_name} in Neo4j");

    let calls_from = edges
        .keys()
        .find(|key| key.kind == "CALLS")
        .expect("CALLS edges in graph");
    let caller = CanonicalId::new(calls_from.from.clone());
    let stored_edges = store
        .get_edges_from(&caller, Some(RelationKind::Calls), 50)
        .await
        .expect("get CALLS edges");
    assert!(!stored_edges.is_empty(), "expected stored CALLS edges");
    assert_eq!(stored_edges[0].kind, RelationKind::Calls);

    store.reset().await.expect("reset failed");
    let after_reset = store
        .find_nodes_by_name(&target_name, 10)
        .await
        .expect("find after reset");
    assert!(after_reset.is_empty(), "graph must be empty after reset");
}

#[tokio::test]
async fn mcp_tools_over_oss_graph() {
    let Some(dir) = ensure_clone(AHO_CORASICK_URL, "aho-corasick") else {
        return;
    };
    let Some((graph, nodes, edges)) = normalize_repo(&dir, "BurntSushi/aho-corasick") else {
        return;
    };

    let call_target = edges
        .values()
        .find(|e| e.kind == RelationKind::Calls && nodes.contains_key(e.to.as_str()))
        .expect("resolved CALLS edge");
    let callee = call_target.to.clone();
    let callee_name = nodes
        .get(callee.as_str())
        .expect("callee node")
        .name
        .clone();

    let store = std::sync::Arc::new(MockStore::new());
    let delta = maps_delta(None, &nodes, &edges, None, "oss".to_string());
    store.apply_delta(&delta).await.expect("seed mock");
    assert_eq!(store.node_count(), graph.nodes.len());

    let api = GraphApi::with_defaults(store.clone());
    let found = api
        .find_symbol(&callee_name, None, 10, None)
        .await
        .expect("find_symbol");
    assert!(
        !found.data.is_empty(),
        "expected {callee_name} via GraphApi"
    );
    assert!(
        found
            .provenance
            .iter()
            .any(|p| p.source == ProvenanceSource::TreeSitter),
        "find_symbol provenance must include tree-sitter"
    );

    let callers = api
        .get_callers(&callee, 2, None)
        .await
        .expect("get_callers");
    assert!(
        !callers.data.is_empty(),
        "expected callers for {callee_name}"
    );
    assert!(
        callers
            .provenance
            .iter()
            .any(|p| p.source == ProvenanceSource::TreeSitter)
    );

    let server = McpServer::new(api);
    let response = server
        .handle_request(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "find_symbol",
                "arguments": {"name": callee_name, "limit": 10}
            }
        }))
        .await;
    let result = &response["result"];
    assert_eq!(result["isError"], serde_json::json!(false));
    let structured = &result["structuredContent"];
    assert!(
        !structured["data"]
            .as_array()
            .expect("data array")
            .is_empty()
    );
    assert_eq!(
        structured["data"][0]["name"],
        serde_json::json!(callee_name)
    );
    assert_eq!(
        structured["provenance"][0]["source"],
        serde_json::json!("TREE_SITTER")
    );
    let text = result["content"][0]["text"].as_str().expect("text content");
    let parsed: serde_json::Value = serde_json::from_str(text).expect("content is json");
    assert_eq!(parsed["data"][0]["name"], serde_json::json!(callee_name));

    let callers_response = server
        .handle_request(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "get_callers",
                "arguments": {"id": callee.as_str(), "depth": 2}
            }
        }))
        .await;
    let callers_result = &callers_response["result"];
    assert_eq!(callers_result["isError"], serde_json::json!(false));
    let callers_structured = &callers_result["structuredContent"];
    assert!(
        !callers_structured["data"]
            .as_array()
            .expect("callers array")
            .is_empty()
    );
    assert!(
        callers_structured["provenance"]
            .as_array()
            .map(|p| !p.is_empty())
            .unwrap_or(false)
    );
}
