use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use ckg_domain::{
    Analyzer, AnalyzerOutput, CanonicalId, CanonicalIdBuilder, ContractBatch, GraphEdge, GraphNode,
    NodeKind, Provenance, ProvenanceSource, RelationKind, SourceLocation,
};
use ckg_git::{CloneConfig, FileDiff, RepoHandle, clone_or_open, detect_language, fetch_updates};
use ckg_graph_delta::{DeltaOp, EdgeKey, EdgeMap, GraphDelta, NodeMap, diff_graphs};
use ckg_links::{contracts_path, is_scannable, load_contracts, save_contracts, scan_file};
use ckg_neo4j_store::{ApplyReport, GraphStore, Neo4jStore};
use ckg_normalizer::{NormalizedGraph, Normalizer};
use ckg_repository_config::RepositoryConfig;
use ckg_tree_sitter_analyzer::TreeSitterAnalyzer;
use tracing::{info, warn};

use crate::state::{load_maps, maps_path, now_label, save_maps};

pub struct BranchAnalysis {
    pub outputs: Vec<AnalyzerOutput>,
    pub analyzer_errors: Vec<String>,
    pub files_analyzed: usize,
}

pub struct BranchReport {
    pub repo: String,
    pub branch: String,
    pub sha: String,
    pub up_to_date: bool,
    pub files_analyzed: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub dangling_edges_skipped: usize,
    pub delta_ops: usize,
    pub apply: Option<ApplyReport>,
}

pub fn repo_cache_path(cache_dir: &Path, clone_url: &str) -> PathBuf {
    CloneConfig::new(clone_url, cache_dir).repo_path()
}

pub fn resolve_repo_path(cache_dir: &Path, repo: &RepositoryConfig) -> PathBuf {
    match &repo.local_path {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => repo_cache_path(cache_dir, &repo.clone_url),
    }
}

pub fn should_index(last: Option<&str>, current: &str, full: bool) -> bool {
    full || last != Some(current)
}

pub fn branch_sha(handle: &RepoHandle, branch: &str) -> Result<Option<String>> {
    if let Some(sha) = handle
        .branch_commit_sha(&format!("origin/{branch}"))
        .with_context(|| format!("failed to resolve origin/{branch}"))?
    {
        return Ok(Some(sha));
    }
    handle
        .branch_commit_sha(branch)
        .with_context(|| format!("failed to resolve branch {branch}"))
}

pub fn open_repo(cache_dir: &Path, repo: &RepositoryConfig) -> Result<RepoHandle> {
    if let Some(local_path) = repo
        .local_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let handle = RepoHandle::open(local_path).with_context(|| {
            format!(
                "failed to open local repository {} at {local_path}",
                repo.id
            )
        })?;
        let _ = fetch_updates(&handle);
        return Ok(handle);
    }
    let config = CloneConfig::new(repo.clone_url.as_str(), cache_dir);
    clone_or_open(&config)
        .with_context(|| format!("failed to clone or open {} ({})", repo.id, repo.clone_url))
}

fn skipped_path(path: &str) -> bool {
    path.split('/')
        .any(|component| matches!(component, "target" | "node_modules" | ".git"))
}

pub fn analyze_files(
    handle: &RepoHandle,
    sha: &str,
    repository: &str,
    allowed_languages: &[String],
) -> Result<BranchAnalysis> {
    let files = handle
        .list_files_at(sha)
        .with_context(|| format!("failed to list files at {sha}"))?;
    analyze_paths(handle, sha, repository, allowed_languages, &files)
}

pub fn analyze_paths(
    handle: &RepoHandle,
    sha: &str,
    repository: &str,
    allowed_languages: &[String],
    paths: &[String],
) -> Result<BranchAnalysis> {
    let analyzer = TreeSitterAnalyzer::new();

    let mut outputs = Vec::new();
    let mut analyzer_errors = Vec::new();
    let mut files_analyzed = 0usize;

    for path in paths {
        if skipped_path(path) {
            continue;
        }
        let Some(language) = detect_language(path) else {
            continue;
        };
        if !allowed_languages.is_empty() && !allowed_languages.iter().any(|l| l == language) {
            continue;
        }
        let Some(content) = handle
            .file_at(sha, path)
            .with_context(|| format!("failed to read {path} at {sha}"))?
        else {
            continue;
        };
        let output = analyzer.analyze_file(repository, sha, path, &content, language);
        for error in &output.errors {
            analyzer_errors.push(format!("{path}: {error}"));
        }
        outputs.push(output);
        files_analyzed += 1;
    }

    Ok(BranchAnalysis {
        outputs,
        analyzer_errors,
        files_analyzed,
    })
}

pub fn normalized_to_maps(graph: &NormalizedGraph) -> (NodeMap, EdgeMap, usize) {
    let nodes: NodeMap = graph
        .nodes
        .values()
        .map(|node| (node.id.to_string(), node.clone()))
        .collect();

    let mut edges = EdgeMap::new();
    let mut skipped = 0usize;
    for edge in graph.edges.values() {
        if nodes.contains_key(edge.from.as_str()) && nodes.contains_key(edge.to.as_str()) {
            edges.insert(ckg_graph_delta::EdgeKey::from_edge(edge), edge.clone());
        } else {
            skipped += 1;
        }
    }
    (nodes, edges, skipped)
}

pub fn maps_delta(
    prev: Option<(&NodeMap, &EdgeMap)>,
    new_nodes: &NodeMap,
    new_edges: &EdgeMap,
    base_revision: Option<String>,
    target_revision: String,
) -> GraphDelta {
    let empty_nodes = NodeMap::new();
    let empty_edges = EdgeMap::new();
    let (old_nodes, old_edges) = match prev {
        Some((nodes, edges)) => (nodes, edges),
        None => (&empty_nodes, &empty_edges),
    };
    let mut delta = diff_graphs(old_nodes, new_nodes, old_edges, new_edges);
    delta.base_revision = base_revision;
    delta.target_revision = Some(target_revision);
    delta
}

pub fn repository_id(github: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(github)
        .symbol("repo")
        .build()
}

pub fn branch_id(github: &str, branch: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(github)
        .symbol("branch")
        .signature(branch)
        .build()
}

pub fn commit_id(github: &str, sha: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(github)
        .symbol("commit")
        .signature(sha)
        .build()
}

pub fn file_id(github: &str, path: &str) -> CanonicalId {
    CanonicalIdBuilder::new()
        .repository(github)
        .symbol("file")
        .signature(path)
        .build()
}

pub fn add_scaffolding(
    nodes: &mut NodeMap,
    edges: &mut EdgeMap,
    github: &str,
    branch: &str,
    sha: &str,
) {
    let prov = Provenance::git();
    let repo_node = GraphNode::new(NodeKind::Repository, repository_id(github), github)
        .with_qualified_name(github)
        .with_provenance(prov.clone())
        .with_property("github", serde_json::Value::String(github.to_string()));
    let branch_node = GraphNode::new(NodeKind::Branch, branch_id(github, branch), branch)
        .with_qualified_name(format!("{github}#{branch}"))
        .with_provenance(prov.clone())
        .with_property("name", serde_json::Value::String(branch.to_string()));
    let commit_node = GraphNode::new(NodeKind::Commit, commit_id(github, sha), sha)
        .with_qualified_name(format!("{github}@{sha}"))
        .with_provenance(prov.clone())
        .with_property("sha", serde_json::Value::String(sha.to_string()));

    edges.insert(
        EdgeKey::new(RelationKind::HasBranch, &repo_node.id, &branch_node.id),
        GraphEdge::new(
            RelationKind::HasBranch,
            repo_node.id.clone(),
            branch_node.id.clone(),
        )
        .with_provenance(prov.clone()),
    );
    edges.insert(
        EdgeKey::new(RelationKind::PointsTo, &branch_node.id, &commit_node.id),
        GraphEdge::new(
            RelationKind::PointsTo,
            branch_node.id.clone(),
            commit_node.id.clone(),
        )
        .with_provenance(prov.clone()),
    );

    let mut file_nodes: Vec<GraphNode> = Vec::new();
    let mut seen_files = std::collections::HashSet::new();
    let mut defined_in: Vec<(CanonicalId, CanonicalId, SourceLocation)> = Vec::new();

    for node in nodes.values() {
        if matches!(
            node.kind,
            NodeKind::Function
                | NodeKind::Method
                | NodeKind::Class
                | NodeKind::Interface
                | NodeKind::Module
                | NodeKind::Package
                | NodeKind::Variable
                | NodeKind::Symbol
        ) {
            if let Some(loc) = &node.location {
                let fid = file_id(github, &loc.path);
                if !nodes.contains_key(fid.as_str()) && seen_files.insert(fid.to_string()) {
                    file_nodes.push(
                        GraphNode::new(NodeKind::File, fid.clone(), loc.path.clone())
                            .with_qualified_name(format!("{github}:{}", loc.path))
                            .with_location(loc.clone())
                            .with_provenance(prov.clone()),
                    );
                }
                defined_in.push((node.id.clone(), fid.clone(), loc.clone()));
            }
            edges.insert(
                EdgeKey::new(RelationKind::ContainsStateOf, &commit_node.id, &node.id),
                GraphEdge::new(
                    RelationKind::ContainsStateOf,
                    commit_node.id.clone(),
                    node.id.clone(),
                )
                .with_provenance(prov.clone()),
            );
        }
    }

    for f in file_nodes {
        edges.insert(
            EdgeKey::new(RelationKind::HasFile, &commit_node.id, &f.id),
            GraphEdge::new(RelationKind::HasFile, commit_node.id.clone(), f.id.clone())
                .with_provenance(prov.clone()),
        );
        nodes.insert(f.id.to_string(), f);
    }

    for (symbol_id, fid, _loc) in defined_in {
        if nodes.contains_key(fid.as_str()) {
            edges.insert(
                EdgeKey::new(RelationKind::DefinedIn, &symbol_id, &fid),
                GraphEdge::new(RelationKind::DefinedIn, symbol_id, fid)
                    .with_provenance(prov.clone()),
            );
        }
    }

    nodes.insert(repo_node.id.to_string(), repo_node);
    nodes.insert(branch_node.id.to_string(), branch_node);
    nodes.insert(commit_node.id.to_string(), commit_node);
}

fn delta_ops_for_full(new_nodes: &NodeMap, new_edges: &EdgeMap, target: String) -> GraphDelta {
    maps_delta(None, new_nodes, new_edges, None, target)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncrementalPlan {
    pub analyze: Vec<String>,
    pub remove_paths: BTreeSet<String>,
}

pub fn incremental_plan(diff: &FileDiff) -> IncrementalPlan {
    let mut analyze: Vec<String> = Vec::new();
    analyze.extend(diff.added.iter().cloned());
    analyze.extend(diff.modified.iter().cloned());
    for (_old, new) in &diff.renamed {
        analyze.push(new.clone());
    }
    analyze.sort();
    analyze.dedup();

    let mut remove_paths: BTreeSet<String> = analyze.iter().cloned().collect();
    remove_paths.extend(diff.deleted.iter().cloned());
    for (old, _new) in &diff.renamed {
        remove_paths.insert(old.clone());
    }
    IncrementalPlan {
        analyze,
        remove_paths,
    }
}

pub fn merge_incremental(
    prev: &(NodeMap, EdgeMap),
    fresh_nodes: &NodeMap,
    fresh_edges: &EdgeMap,
    remove_paths: &BTreeSet<String>,
    github: &str,
    branch: &str,
    sha: &str,
) -> (NodeMap, EdgeMap) {
    let mut removed: HashSet<String> = HashSet::new();
    for node in prev.0.values() {
        let path_changed = node
            .location
            .as_ref()
            .is_some_and(|loc| remove_paths.contains(&loc.path));
        if path_changed || matches!(node.kind, NodeKind::Commit) {
            removed.insert(node.id.to_string());
        }
    }

    let mut nodes: NodeMap = prev
        .0
        .iter()
        .filter(|(id, _)| !removed.contains(id.as_str()))
        .map(|(id, node)| (id.clone(), node.clone()))
        .collect();
    for (id, node) in fresh_nodes {
        nodes.insert(id.clone(), node.clone());
    }

    let mut edges = EdgeMap::new();
    for (key, edge) in &prev.1 {
        if nodes.contains_key(edge.from.as_str()) && nodes.contains_key(edge.to.as_str()) {
            edges.insert(key.clone(), edge.clone());
        }
    }
    for (key, edge) in fresh_edges {
        if nodes.contains_key(edge.from.as_str()) && nodes.contains_key(edge.to.as_str()) {
            edges.insert(key.clone(), edge.clone());
        }
    }

    add_scaffolding(&mut nodes, &mut edges, github, branch, sha);
    (nodes, edges)
}

fn collect_evidence(handle: &RepoHandle, sha: &str, paths: &[String]) -> Result<ContractBatch> {
    let mut batch = ContractBatch::default();
    for path in paths {
        if skipped_path(path) || !is_scannable(path) {
            continue;
        }
        let Some(content) = handle
            .file_at(sha, path)
            .with_context(|| format!("failed to read {path} at {sha}"))?
        else {
            continue;
        };
        batch.merge(scan_file(path, &content));
    }
    Ok(batch)
}

/// Persist the per-branch contract inventory at `path`.
///
/// Full mode rewrites the file from `fresh` alone (a re-index replaces the
/// whole inventory). Incremental mode drops records of deleted files, supersedes
/// records of re-analyzed files, and keeps everything untouched.
pub fn persist_contracts(
    path: &Path,
    incremental: bool,
    fresh: &ContractBatch,
    removed: &[String],
) -> Result<()> {
    if !incremental {
        save_contracts(path, fresh)
            .with_context(|| format!("failed to save contracts {}", path.display()))?;
        return Ok(());
    }
    let stored = load_contracts(path)
        .with_context(|| format!("failed to load contracts {}", path.display()))?;
    if stored.is_none() && fresh.is_empty() {
        return Ok(());
    }
    let mut batch = stored.unwrap_or_default();
    batch.remove_files(removed);
    batch.replace_files(fresh.clone());
    save_contracts(path, &batch)
        .with_context(|| format!("failed to save contracts {}", path.display()))?;
    Ok(())
}

pub async fn index_branch(
    handle: &RepoHandle,
    store: &dyn GraphStore,
    cache_dir: &Path,
    repo: &RepositoryConfig,
    branch: &str,
    last_sha: Option<&str>,
    prev_maps: Option<(NodeMap, EdgeMap)>,
    full: bool,
    state: &mut crate::state::IndexState,
) -> Result<BranchReport> {
    let sha = branch_sha(handle, branch)?
        .ok_or_else(|| anyhow!("branch {branch} not found for repository {}", repo.id))?;

    if !should_index(last_sha, &sha, full) {
        info!(
            repo = %repo.id,
            branch = %branch,
            sha = %sha,
            "already indexed, skipping (use --full to force)"
        );
        return Ok(BranchReport {
            repo: repo.id.clone(),
            branch: branch.to_string(),
            sha,
            up_to_date: true,
            files_analyzed: 0,
            node_count: 0,
            edge_count: 0,
            dangling_edges_skipped: 0,
            delta_ops: 0,
            apply: None,
        });
    }

    if !repo.indexing.analyzers.iter().any(|a| a == "tree-sitter") {
        bail!(
            "repository {} does not enable the tree-sitter analyzer (analyzers: {:?})",
            repo.id,
            repo.indexing.analyzers
        );
    }
    if repo.indexing.scip_enabled || repo.indexing.joern_enabled {
        warn!(
            repo = %repo.id,
            "scip/joern analyzers are enabled but not wired in this MVP; using tree-sitter only"
        );
    }

    let incremental = !full && last_sha.is_some() && prev_maps.is_some();
    let mut built = None;
    let mut fresh = ContractBatch::default();
    let mut removed_contract_paths: Option<Vec<String>> = None;

    if incremental {
        let last = last_sha.expect("incremental requires last_sha");
        let prev = prev_maps.as_ref().expect("incremental requires prev maps");
        match handle.diff_files(last, &sha) {
            Ok(diff) => {
                let plan = incremental_plan(&diff);
                let analysis = analyze_paths(
                    handle,
                    &sha,
                    repo.github_or_id(),
                    &repo.indexing.languages,
                    &plan.analyze,
                )?;
                for error in &analysis.analyzer_errors {
                    warn!(repo = %repo.id, branch = %branch, "analyzer error (partial provenance): {error}");
                }
                for output in &analysis.outputs {
                    fresh.merge(output.contracts.clone());
                }
                fresh.merge(collect_evidence(handle, &sha, &plan.analyze)?);
                removed_contract_paths = Some(plan.remove_paths.iter().cloned().collect());
                let graph = Normalizer::normalize(analysis.outputs);
                for error in &graph.errors {
                    warn!(repo = %repo.id, branch = %branch, "normalization warning: {error}");
                }
                let (fresh_nodes, fresh_edges, dangling) = normalized_to_maps(&graph);
                if dangling > 0 {
                    warn!(
                        repo = %repo.id,
                        branch = %branch,
                        "skipped {dangling} relationship(s) with missing endpoints"
                    );
                }
                let (new_nodes, new_edges) = merge_incremental(
                    prev,
                    &fresh_nodes,
                    &fresh_edges,
                    &plan.remove_paths,
                    repo.github_or_id(),
                    branch,
                    &sha,
                );
                info!(
                    repo = %repo.id,
                    branch = %branch,
                    sha = %sha,
                    changed = plan.analyze.len(),
                    removed = plan.remove_paths.len(),
                    "incremental analysis"
                );
                built = Some((analysis.files_analyzed, new_nodes, new_edges, dangling));
            }
            Err(err) => {
                warn!(
                    repo = %repo.id,
                    branch = %branch,
                    "diff_files failed ({err:#}); falling back to full analysis"
                );
            }
        }
    }

    let used_incremental = built.is_some();
    let (files_analyzed, new_nodes, new_edges, dangling) = match built {
        Some(built) => built,
        None => {
            let analysis =
                analyze_files(handle, &sha, repo.github_or_id(), &repo.indexing.languages)?;
            for error in &analysis.analyzer_errors {
                warn!(repo = %repo.id, branch = %branch, "analyzer error (partial provenance): {error}");
            }

            for output in &analysis.outputs {
                fresh.merge(output.contracts.clone());
            }
            let evidence_paths = handle
                .list_files_at(&sha)
                .with_context(|| format!("failed to list files at {sha}"))?;
            fresh.merge(collect_evidence(handle, &sha, &evidence_paths)?);

            let graph = Normalizer::normalize(analysis.outputs);
            for error in &graph.errors {
                warn!(repo = %repo.id, branch = %branch, "normalization warning: {error}");
            }

            let (mut nodes, mut edges, dangling) = normalized_to_maps(&graph);
            add_scaffolding(&mut nodes, &mut edges, repo.github_or_id(), branch, &sha);
            if dangling > 0 {
                warn!(
                    repo = %repo.id,
                    branch = %branch,
                    "skipped {dangling} relationship(s) with missing endpoints"
                );
            }
            (analysis.files_analyzed, nodes, edges, dangling)
        }
    };

    let prev_ref = prev_maps.as_ref().map(|(n, e)| (n, e));
    let use_prev = !full;
    let delta = if use_prev {
        maps_delta(
            prev_ref,
            &new_nodes,
            &new_edges,
            last_sha.map(str::to_string),
            sha.clone(),
        )
    } else {
        delta_ops_for_full(&new_nodes, &new_edges, sha.clone())
    };

    info!(
        repo = %repo.id,
        branch = %branch,
        sha = %sha,
        files = files_analyzed,
        nodes = new_nodes.len(),
        edges = new_edges.len(),
        delta_ops = delta.len(),
        "applying graph delta"
    );

    let report = store
        .apply_delta(&delta)
        .await
        .with_context(|| format!("failed to apply delta for {}@{}", repo.id, branch))?;

    for error in &report.errors {
        warn!(repo = %repo.id, branch = %branch, "apply warning: {error}");
    }

    save_maps(
        &maps_path(cache_dir, &repo.id, branch),
        &new_nodes,
        &new_edges,
    )?;
    persist_contracts(
        &contracts_path(cache_dir, &repo.id, branch),
        used_incremental,
        &fresh,
        removed_contract_paths.as_deref().unwrap_or_default(),
    )
    .with_context(|| format!("failed to persist contracts for {}@{}", repo.id, branch))?;
    state.record(&repo.id, branch, sha.clone(), now_label());

    Ok(BranchReport {
        repo: repo.id.clone(),
        branch: branch.to_string(),
        sha,
        up_to_date: false,
        files_analyzed,
        node_count: new_nodes.len(),
        edge_count: new_edges.len(),
        dangling_edges_skipped: dangling,
        delta_ops: delta.len(),
        apply: Some(report),
    })
}

pub fn connect_store(uri: &str, user: &str, password: &str, database: &str) -> Result<Neo4jStore> {
    Neo4jStore::connect(uri, user, password, database)
        .with_context(|| format!("failed to connect to Neo4j at {uri}"))
}

pub fn load_prev_maps(
    cache_dir: &Path,
    repo: &str,
    branch: &str,
) -> Result<Option<(NodeMap, EdgeMap)>> {
    load_maps(&maps_path(cache_dir, repo, branch))
}

pub fn tree_sitter_provenance(output: &AnalyzerOutput) -> bool {
    output
        .provenance
        .iter()
        .any(|p| p.source == ProvenanceSource::TreeSitter)
}

pub fn count_ops(delta: &GraphDelta) -> (usize, usize) {
    let creates = delta
        .ops
        .iter()
        .filter(|op| matches!(op, DeltaOp::CreateNode(_) | DeltaOp::CreateRelationship(_)))
        .count();
    (creates, delta.ops.len() - creates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::{ConsumedContract, Mechanism, ProvidedEndpoint};

    fn provide(file: &str, route: &str) -> ProvidedEndpoint {
        ProvidedEndpoint {
            file: file.to_string(),
            mechanism: Mechanism::Rest,
            http_method: "GET".to_string(),
            route: route.to_string(),
            service: String::new(),
            method: String::new(),
            handler_id: CanonicalId::from(format!("{file}-handler")),
            framework: "test".to_string(),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    fn consume(file: &str, route: &str) -> ConsumedContract {
        ConsumedContract {
            file: file.to_string(),
            mechanism: Mechanism::Rest,
            http_method: "GET".to_string(),
            route: route.to_string(),
            service: String::new(),
            method: String::new(),
            caller_id: CanonicalId::from(format!("{file}-caller")),
            target_hint: String::new(),
            framework: "test".to_string(),
            language: "rust".to_string(),
            location: None,
            properties: Default::default(),
        }
    }

    fn batch(provides: &[(&str, &str)]) -> ContractBatch {
        let mut batch = ContractBatch::default();
        for (file, route) in provides {
            batch.provides.push(provide(file, route));
        }
        batch
    }

    #[test]
    fn full_persist_replaces_the_whole_inventory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = contracts_path(tmp.path(), "org/repo", "main");
        save_contracts(&path, &batch(&[("old.rs", "/old"), ("keep.rs", "/keep")])).expect("seed");

        let fresh = batch(&[("new.rs", "/new")]);
        persist_contracts(&path, false, &fresh, &[]).expect("persist");

        let stored = load_contracts(&path).expect("load").expect("stored");
        assert_eq!(stored, fresh, "full mode overwrites everything");
        assert!(stored.provides.iter().all(|p| p.file == "new.rs"));
    }

    #[test]
    fn full_persist_writes_even_when_fresh_is_empty() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = contracts_path(tmp.path(), "org/repo", "main");
        save_contracts(&path, &batch(&[("old.rs", "/old")])).expect("seed");

        persist_contracts(&path, false, &ContractBatch::default(), &[]).expect("persist");

        let stored = load_contracts(&path).expect("load").expect("stored");
        assert!(stored.is_empty(), "re-index clears removed contracts");
    }

    #[test]
    fn incremental_persist_removes_deleted_and_replaces_reanalyzed() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = contracts_path(tmp.path(), "org/repo", "main");
        let mut previous = batch(&[
            ("reanalyzed.rs", "/old-route"),
            ("untouched.rs", "/keep"),
            ("deleted.rs", "/gone"),
        ]);
        previous
            .consumes
            .push(consume("deleted.rs", "/gone-consume"));
        save_contracts(&path, &previous).expect("seed");

        let fresh = batch(&[("reanalyzed.rs", "/new-route")]);
        let removed = vec!["deleted.rs".to_string()];
        persist_contracts(&path, true, &fresh, &removed).expect("persist");

        let stored = load_contracts(&path).expect("load").expect("stored");
        assert_eq!(stored.provides.len(), 2);
        assert!(
            stored
                .provides
                .iter()
                .any(|p| p.file == "reanalyzed.rs" && p.route == "/new-route"),
            "reanalyzed records supersede the stored ones"
        );
        assert!(
            stored.provides.iter().any(|p| p.file == "untouched.rs"),
            "untouched records survive"
        );
        assert!(
            !stored.provides.iter().any(|p| p.file == "deleted.rs") && stored.consumes.is_empty(),
            "deleted files are dropped across record kinds"
        );
    }

    #[test]
    fn incremental_persist_seeds_missing_file_and_skips_when_nothing_known() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = contracts_path(tmp.path(), "org/repo", "main");

        let fresh = batch(&[("new.rs", "/new")]);
        persist_contracts(&path, true, &fresh, &[]).expect("persist");
        assert_eq!(load_contracts(&path).expect("load").expect("stored"), fresh);

        let empty_path = contracts_path(tmp.path(), "org/other", "main");
        persist_contracts(&empty_path, true, &ContractBatch::default(), &[]).expect("persist");
        assert!(
            !empty_path.exists(),
            "nothing stored and nothing fresh leaves no file behind"
        );
    }

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
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
    fn collect_evidence_reads_scannable_paths_and_gates_the_rest() {
        let repo_dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(repo_dir.path().join("src")).expect("mkdir");
        std::fs::write(repo_dir.path().join("src/main.rs"), "fn main() {}\n").expect("write");
        std::fs::write(
            repo_dir.path().join(".env"),
            "ORDERS_BASE_URL=https://api.orders.internal\n",
        )
        .expect("write");
        git(repo_dir.path(), &["init", "-q"]);
        git(repo_dir.path(), &["add", "-A"]);
        git(repo_dir.path(), &["commit", "-q", "-m", "init"]);
        let sha = git(repo_dir.path(), &["rev-parse", "HEAD"]);

        let handle = RepoHandle::open(repo_dir.path()).expect("open repo");
        let paths = vec![
            "src/main.rs".to_string(),
            ".env".to_string(),
            "target/generated.yml".to_string(),
        ];

        let evidence = collect_evidence(&handle, &sha, &paths).expect("collect");
        assert!(
            !evidence.evidence.is_empty(),
            "scannable file must contribute evidence"
        );
        assert!(
            evidence.evidence.iter().all(|record| record.file == ".env"),
            "code files and skipped paths contribute nothing: {:?}",
            evidence.evidence
        );
        assert!(
            evidence
                .evidence
                .iter()
                .any(|record| record.value == "https://api.orders.internal")
        );

        let code_only =
            collect_evidence(&handle, &sha, &["src/main.rs".to_string()]).expect("collect");
        assert!(code_only.is_empty(), "non-scannable path yields nothing");
    }
}
