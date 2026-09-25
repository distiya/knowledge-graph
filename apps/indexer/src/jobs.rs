use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ckg_domain::ContractBatch;
use ckg_links::{
    BranchHead, compute_links, contracts_path, diff_link_sets, link_set_path, load_contracts,
    load_link_set, save_link_set,
};
use ckg_neo4j_store::GraphStore;
use ckg_repository_config::{RepositoryConfig, WorkspaceConfig};
use clap::Args;
use tracing::{error, info, warn};

use crate::pipeline::{self, load_prev_maps, open_repo};
use crate::state::{IndexState, state_path};

pub use crate::pipeline::should_index;

#[derive(Debug, Clone, Args)]
pub struct Neo4jArgs {
    /// Neo4j bolt/neo4j URI
    #[arg(long)]
    pub neo4j_uri: Option<String>,
    /// Neo4j username
    #[arg(long)]
    pub neo4j_user: Option<String>,
    /// Neo4j password
    #[arg(long)]
    pub neo4j_password: Option<String>,
    /// Neo4j database name
    #[arg(long, default_value = "neo4j")]
    pub neo4j_database: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neo4jSettings {
    pub uri: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Default for Neo4jSettings {
    fn default() -> Self {
        Self {
            uri: "bolt://localhost:7687".to_string(),
            user: "neo4j".to_string(),
            password: "testpassword".to_string(),
            database: "neo4j".to_string(),
        }
    }
}

fn env_or(flag: Option<String>, var: &str, default: &str) -> String {
    flag.unwrap_or_else(|| {
        std::env::var(var)
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| default.to_string())
    })
}

impl Neo4jArgs {
    pub fn settings(&self) -> Neo4jSettings {
        Neo4jSettings {
            uri: env_or(self.neo4j_uri.clone(), "NEO4J_URI", "bolt://localhost:7687"),
            user: env_or(self.neo4j_user.clone(), "NEO4J_USER", "neo4j"),
            password: env_or(
                self.neo4j_password.clone(),
                "NEO4J_PASSWORD",
                "testpassword",
            ),
            database: self.neo4j_database.clone(),
        }
    }
}

pub fn select_repos(
    workspace: &WorkspaceConfig,
    repo_filter: Option<&str>,
) -> Result<Vec<RepositoryConfig>> {
    let mut repos: Vec<RepositoryConfig> = workspace
        .repositories
        .iter()
        .filter(|repo| repo.enabled)
        .cloned()
        .collect();
    if let Some(id) = repo_filter {
        repos.retain(|repo| repo.id == id);
        if repos.is_empty() {
            match workspace.repositories.iter().find(|r| r.id == id) {
                Some(repo) if !repo.enabled => {
                    return Err(anyhow!("repository '{id}' is disabled in configuration"));
                }
                _ => return Err(anyhow!("repository id '{id}' not found in configuration")),
            }
        }
    }
    Ok(repos)
}

pub fn branches_for(
    workspace: &WorkspaceConfig,
    repo: &RepositoryConfig,
    branch_filter: Option<&str>,
) -> Vec<String> {
    match branch_filter {
        Some(branch) => vec![branch.to_string()],
        None => workspace.resolved_branches(repo),
    }
}

#[derive(Debug, Default, Clone)]
pub struct PassOutcome {
    pub indexed: usize,
    pub up_to_date: usize,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkReport {
    pub edges: usize,
    pub topics: usize,
    pub ops_applied: usize,
}

/// Recompute the workspace link set from the persisted per-branch contract
/// inventories and apply the difference to the store.
pub async fn run_link_pass(
    store: &dyn GraphStore,
    workspace: &WorkspaceConfig,
    cache_dir: &Path,
    state: &IndexState,
) -> Result<LinkReport> {
    let mut inputs: Vec<(BranchHead, ContractBatch)> = Vec::new();
    for repo in workspace.repositories.iter().filter(|repo| repo.enabled) {
        for branch in workspace.resolved_branches(repo) {
            let Some(sha) = state.last_sha(&repo.id, &branch) else {
                continue;
            };
            let path = contracts_path(cache_dir, &repo.id, &branch);
            let Some(batch) = load_contracts(&path)
                .with_context(|| format!("failed to load contracts {}", path.display()))?
            else {
                continue;
            };
            inputs.push((
                BranchHead {
                    repo_id: repo.id.clone(),
                    github: repo.github_or_id().to_string(),
                    branch: branch.clone(),
                    sha: sha.to_string(),
                    commit_id: pipeline::commit_id(repo.github_or_id(), sha),
                },
                batch,
            ));
        }
    }

    let new = compute_links(workspace, &inputs);
    let link_path = link_set_path(cache_dir);
    let old = load_link_set(&link_path)
        .with_context(|| format!("failed to load link set {}", link_path.display()))?;

    if new == old {
        return Ok(LinkReport {
            edges: new.edges.len(),
            topics: new.topics.len(),
            ops_applied: 0,
        });
    }

    let delta = diff_link_sets(&old, &new);
    let ops_applied = delta.ops.len();
    let apply = store
        .apply_delta(&delta)
        .await
        .with_context(|| "failed to apply link delta")?;
    for error in &apply.errors {
        warn!("link apply warning: {error}");
    }
    if !apply.is_success() {
        bail!(
            "link delta apply reported {} error(s): {}",
            apply.errors.len(),
            apply.errors.join("; ")
        );
    }
    save_link_set(&link_path, &new)
        .with_context(|| format!("failed to save link set {}", link_path.display()))?;

    Ok(LinkReport {
        edges: new.edges.len(),
        topics: new.topics.len(),
        ops_applied,
    })
}

pub async fn run_index_pass(
    store: &dyn GraphStore,
    config_path: &Path,
    cache_dir: &Path,
    repo_filter: Option<&str>,
    branch_filter: Option<&str>,
    full: bool,
) -> Result<PassOutcome> {
    let workspace = WorkspaceConfig::load(config_path)
        .with_context(|| format!("failed to load config {}", config_path.display()))?;
    let repos = select_repos(&workspace, repo_filter)?;

    if repos.is_empty() {
        warn!("no enabled repositories selected");
        return Ok(PassOutcome::default());
    }

    std::fs::create_dir_all(cache_dir)
        .with_context(|| format!("failed to create {}", cache_dir.display()))?;

    let state_file = state_path(cache_dir);
    let mut index_state = IndexState::load(&state_file)?;
    let mut outcome = PassOutcome::default();

    for repo in &repos {
        let branches = branches_for(&workspace, repo, branch_filter);
        if branches.is_empty() {
            warn!(repo = %repo.id, "no monitored branches; skipping");
            continue;
        }

        let handle = match open_repo(cache_dir, repo) {
            Ok(handle) => handle,
            Err(err) => {
                outcome.failures.push(format!("{}: {err:#}", repo.id));
                error!(repo = %repo.id, "clone/open failed: {err:#}");
                continue;
            }
        };

        for branch in &branches {
            let last_sha = index_state.last_sha(&repo.id, branch).map(str::to_string);
            let prev_maps = load_prev_maps(cache_dir, &repo.id, branch)?;

            match pipeline::index_branch(
                &handle,
                store,
                cache_dir,
                repo,
                branch,
                last_sha.as_deref(),
                prev_maps,
                full,
                &mut index_state,
            )
            .await
            {
                Ok(report) => {
                    if report.up_to_date {
                        outcome.up_to_date += 1;
                        info!(
                            repo = %report.repo,
                            branch = %report.branch,
                            sha = %report.sha,
                            "up to date"
                        );
                        continue;
                    }
                    outcome.indexed += 1;
                    if let Some(apply) = report.apply.as_ref() {
                        info!(
                            repo = %report.repo,
                            branch = %report.branch,
                            sha = %report.sha,
                            files = report.files_analyzed,
                            nodes = report.node_count,
                            edges = report.edge_count,
                            created_nodes = apply.created_nodes,
                            updated_nodes = apply.updated_nodes,
                            deleted_nodes = apply.deleted_nodes,
                            created_rels = apply.created_rels,
                            deleted_rels = apply.deleted_rels,
                            "indexed"
                        );
                        if !apply.is_success() {
                            outcome.failures.push(format!(
                                "{}@{}: apply reported {} error(s): {}",
                                report.repo,
                                report.branch,
                                apply.errors.len(),
                                apply.errors.join("; ")
                            ));
                        }
                    }
                }
                Err(err) => {
                    outcome
                        .failures
                        .push(format!("{}@{branch}: {err:#}", repo.id));
                    error!(repo = %repo.id, branch = %branch, "index failed: {err:#}");
                }
            }
        }
    }

    if workspace.links.enabled {
        match run_link_pass(store, &workspace, cache_dir, &index_state).await {
            Ok(report) => info!(
                edges = report.edges,
                topics = report.topics,
                ops = report.ops_applied,
                "link pass complete"
            ),
            Err(err) => {
                outcome.failures.push(format!("link pass: {err:#}"));
                error!("link pass failed: {err:#}");
            }
        }
    }

    index_state.save(&state_file)?;
    info!("state saved to {}", state_file.display());
    Ok(outcome)
}

pub async fn run_watch(
    store: &dyn GraphStore,
    config_path: &Path,
    cache_dir: &Path,
    repo_filter: Option<&str>,
    branch_filter: Option<&str>,
    interval_secs: u64,
) -> Result<()> {
    let interval = Duration::from_secs(interval_secs.max(1));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    info!(
        interval_secs = interval.as_secs(),
        "watch started; press ctrl-c to stop"
    );

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match run_index_pass(store, config_path, cache_dir, repo_filter, branch_filter, false).await {
                    Ok(outcome) => {
                        if !outcome.failures.is_empty() {
                            for failure in &outcome.failures {
                                error!("watch failure: {failure}");
                            }
                        }
                        info!(
                            indexed = outcome.indexed,
                            skipped = outcome.up_to_date,
                            failures = outcome.failures.len(),
                            "watch tick complete"
                        );
                    }
                    Err(err) => error!("watch tick failed: {err:#}"),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("watch interrupted; shutting down");
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_index_decides_skip_vs_run() {
        assert!(should_index(None, "abc", false), "first index always runs");
        assert!(
            should_index(Some("abc"), "def", false),
            "changed sha reindexes"
        );
        assert!(
            !should_index(Some("abc"), "abc", false),
            "unchanged sha skips"
        );
        assert!(
            should_index(Some("abc"), "abc", true),
            "full forces reindex"
        );
        assert!(should_index(Some("abc"), "def", true));
    }

    #[test]
    fn select_repos_filters_enabled_and_reports_missing() {
        use ckg_repository_config::RepositoryConfig;
        let mut workspace = WorkspaceConfig::default();
        workspace.repositories.push(
            RepositoryConfig::new("on", "acme/on", "https://example.com/on.git")
                .with_branches(["main"]),
        );
        workspace.repositories.push(
            RepositoryConfig::new("off", "acme/off", "https://example.com/off.git")
                .with_branches(["main"])
                .with_enabled(false),
        );

        let all = select_repos(&workspace, None).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "on");

        let filtered = select_repos(&workspace, Some("on")).unwrap();
        assert_eq!(filtered.len(), 1);

        let err = select_repos(&workspace, Some("off")).unwrap_err();
        assert!(err.to_string().contains("disabled"));

        let err = select_repos(&workspace, Some("nope")).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn branches_for_uses_workspace_defaults_when_monitored_empty() {
        use ckg_repository_config::RepositoryConfig;
        let mut workspace = WorkspaceConfig::default();
        workspace.default_branches = vec!["main".into(), "develop".into()];
        let repo = RepositoryConfig::new("r", "acme/r", "https://example.com/r.git")
            .with_branches(Vec::<String>::new());

        assert_eq!(
            branches_for(&workspace, &repo, None),
            vec!["main".to_string(), "develop".to_string()]
        );
        assert_eq!(
            branches_for(&workspace, &repo, Some("hotfix")),
            vec!["hotfix".to_string()]
        );
    }
}
