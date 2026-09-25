use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use ckg_indexer::jobs::{self, Neo4jArgs};
use ckg_indexer::pipeline;
use ckg_indexer::server::{self, ServeArgs};
use ckg_indexer::state::{IndexState, state_path};
use ckg_repository_config::WorkspaceConfig;
use clap::{Args, Parser, Subcommand};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "ckg-indexer",
    version,
    about = "Index repositories into the enterprise code knowledge graph"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the indexing pipeline for enabled repositories
    Index(IndexArgs),
    /// Continuously watch configured repositories and index on change
    Watch(WatchArgs),
    /// Serve the admin API and static UI
    Serve(ServeArgs),
    /// Show configured repositories, branches, and last indexed commits
    Status(StatusArgs),
}

#[derive(Debug, Args)]
struct ConfigArgs {
    /// Path to the workspace configuration file
    #[arg(long, default_value = "config/workspace.example.toml")]
    config: PathBuf,
    /// Cache directory for clones, index state, and snapshots
    #[arg(long, default_value = ".ckg-cache")]
    cache_dir: PathBuf,
    /// Only index this repository id
    #[arg(long)]
    repo: Option<String>,
    /// Only index this branch (overrides monitored branch lists)
    #[arg(long)]
    branch: Option<String>,
}

#[derive(Debug, Args)]
struct IndexArgs {
    #[command(flatten)]
    config: ConfigArgs,
    #[command(flatten)]
    neo4j: Neo4jArgs,
    /// Reindex even when the branch sha is unchanged
    #[arg(long)]
    full: bool,
}

#[derive(Debug, Args)]
struct WatchArgs {
    #[command(flatten)]
    config: ConfigArgs,
    #[command(flatten)]
    neo4j: Neo4jArgs,
    /// Poll interval in seconds
    #[arg(long, default_value_t = 30)]
    interval: u64,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[command(flatten)]
    config: ConfigArgs,
}

async fn run_index(args: IndexArgs) -> Result<()> {
    let settings = args.neo4j.settings();
    let store = pipeline::connect_store(
        &settings.uri,
        &settings.user,
        &settings.password,
        &settings.database,
    )?;
    info!(uri = %settings.uri, "connected to Neo4j");

    let outcome = jobs::run_index_pass(
        &store,
        &args.config.config,
        &args.config.cache_dir,
        args.config.repo.as_deref(),
        args.config.branch.as_deref(),
        args.full,
    )
    .await?;

    if !outcome.failures.is_empty() {
        for failure in &outcome.failures {
            error!("hard failure: {failure}");
        }
        return Err(anyhow!(
            "indexing finished with {} hard failure(s)",
            outcome.failures.len()
        ));
    }

    info!("indexed {} repo/branch pair(s)", outcome.indexed);
    Ok(())
}

async fn run_watch(args: WatchArgs) -> Result<()> {
    let settings = args.neo4j.settings();
    let store = pipeline::connect_store(
        &settings.uri,
        &settings.user,
        &settings.password,
        &settings.database,
    )?;
    info!(uri = %settings.uri, "connected to Neo4j");

    jobs::run_watch(
        &store,
        &args.config.config,
        &args.config.cache_dir,
        args.config.repo.as_deref(),
        args.config.branch.as_deref(),
        args.interval,
    )
    .await
}

fn run_status(args: StatusArgs) -> Result<()> {
    let workspace = WorkspaceConfig::load(&args.config.config)
        .with_context(|| format!("failed to load config {}", args.config.config.display()))?;
    let state = IndexState::load(&state_path(&args.config.cache_dir))?;

    println!(
        "{:<16} {:<10} {:<12} {:<42} {:<42}",
        "REPOSITORY", "ENABLED", "BRANCH", "LAST INDEXED", "CURRENT"
    );

    for repo in &workspace.repositories {
        if let Some(filter) = args.config.repo.as_deref() {
            if repo.id != filter {
                continue;
            }
        }
        let branches = jobs::branches_for(&workspace, repo, args.config.branch.as_deref());
        let cache_path = pipeline::resolve_repo_path(&args.config.cache_dir, repo);
        let handle = ckg_git::RepoHandle::open(&cache_path).ok();
        if let Some(handle) = &handle {
            let _ = ckg_git::fetch_updates(handle);
        }

        for branch in &branches {
            let last = state
                .last_sha(&repo.id, branch)
                .map(str::to_string)
                .unwrap_or_else(|| "-".to_string());
            let current = match &handle {
                Some(handle) => match pipeline::branch_sha(handle, branch) {
                    Ok(Some(sha)) => sha,
                    Ok(None) => "- (branch not found)".to_string(),
                    Err(err) => format!("- (error: {err:#})"),
                },
                None => "- (not cloned)".to_string(),
            };
            println!(
                "{:<16} {:<10} {:<12} {:<42} {:<42}",
                repo.id,
                if repo.enabled { "yes" } else { "no" },
                branch,
                last,
                current
            );
        }

        if branches.is_empty() {
            println!(
                "{:<16} {:<10} {:<12} {:<42} {:<42}",
                repo.id,
                if repo.enabled { "yes" } else { "no" },
                "-",
                "-",
                "-"
            );
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Command::Index(args) => run_index(args).await,
        Command::Watch(args) => run_watch(args).await,
        Command::Serve(args) => server::serve(args).await,
        Command::Status(args) => run_status(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}
