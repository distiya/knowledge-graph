use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::Json;
use axum::Router;
use axum::extract::{Path as PathExtractor, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use ckg_git::RepoHandle;
use ckg_repository_config::{RepositoryConfig, WorkspaceConfig};
use clap::Args;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info, warn};

use crate::jobs::{Neo4jArgs, Neo4jSettings, run_index_pass};
use crate::pipeline::{branch_sha, resolve_repo_path};
use crate::state::{IndexState, state_path};

const INDEX_HTML: &str = include_str!("../static/index.html");

#[derive(Debug, Clone, Args)]
pub struct ServeArgs {
    /// Path to the workspace configuration file
    #[arg(long, default_value = "config/workspace.example.toml")]
    pub config: PathBuf,
    /// Cache directory for clones, index state, and snapshots
    #[arg(long, default_value = ".ckg-cache")]
    pub cache_dir: PathBuf,
    /// TCP port to listen on (binds 0.0.0.0)
    #[arg(long, default_value_t = 8080)]
    pub port: u16,
    #[command(flatten)]
    pub neo4j: Neo4jArgs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexJob {
    pub repo_id: String,
    pub branch: Option<String>,
}

pub struct AppState {
    pub config_path: PathBuf,
    pub cache_dir: PathBuf,
    pub workspace: RwLock<WorkspaceConfig>,
    pub neo4j: Neo4jSettings,
    pub job_tx: mpsc::UnboundedSender<IndexJob>,
    pub active: Mutex<HashSet<String>>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(
        config_path: PathBuf,
        cache_dir: PathBuf,
        workspace: WorkspaceConfig,
        neo4j: Neo4jSettings,
    ) -> (SharedState, mpsc::UnboundedReceiver<IndexJob>) {
        let (job_tx, job_rx) = mpsc::unbounded_channel();
        let state = Arc::new(AppState {
            config_path,
            cache_dir,
            workspace: RwLock::new(workspace),
            neo4j,
            job_tx,
            active: Mutex::new(HashSet::new()),
        });
        (state, job_rx)
    }
}

pub fn load_or_init_workspace(config_path: &Path) -> Result<WorkspaceConfig> {
    if config_path.exists() {
        return WorkspaceConfig::load(config_path)
            .with_context(|| format!("failed to load config {}", config_path.display()));
    }
    let workspace = WorkspaceConfig::default();
    workspace
        .save(config_path)
        .with_context(|| format!("failed to create empty config {}", config_path.display()))?;
    warn!(
        "config {} was missing; created an empty workspace configuration",
        config_path.display()
    );
    Ok(workspace)
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

fn bad_request(err: impl std::fmt::Display) -> Response {
    error_response(StatusCode::BAD_REQUEST, err.to_string())
}

fn save_failed(err: impl std::fmt::Display) -> Response {
    error_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("failed to save config: {err}"),
    )
}

fn try_lock_active(state: &AppState, repo_id: &str) -> bool {
    let mut active = state.active.lock().expect("active index lock");
    active.insert(repo_id.to_string())
}

fn release_active(state: &AppState, repo_id: &str) {
    state
        .active
        .lock()
        .expect("active index lock")
        .remove(repo_id);
}

pub async fn index_page() -> Html<&'static str> {
    Html(INDEX_HTML)
}

pub async fn list_repos(State(state): State<SharedState>) -> Json<Vec<RepositoryConfig>> {
    let workspace = state.workspace.read().await;
    Json(workspace.repositories.clone())
}

pub async fn add_repo(
    State(state): State<SharedState>,
    Json(repo): Json<RepositoryConfig>,
) -> Result<Response, Response> {
    let mut workspace = state.workspace.write().await;
    workspace
        .add_repo(repo.clone())
        .map_err(|err| bad_request(err))?;
    workspace.save(&state.config_path).map_err(save_failed)?;
    Ok((StatusCode::CREATED, Json(repo)).into_response())
}

pub async fn delete_repo(
    State(state): State<SharedState>,
    PathExtractor(id): PathExtractor<String>,
) -> Result<Response, Response> {
    let mut workspace = state.workspace.write().await;
    if workspace.remove_repo(&id).is_none() {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            format!("repository '{id}' not found"),
        ));
    }
    workspace.save(&state.config_path).map_err(save_failed)?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Debug, Deserialize)]
pub struct PatchRepoBody {
    pub enabled: Option<bool>,
    pub monitored_branches: Option<Vec<String>>,
}

pub async fn patch_repo(
    State(state): State<SharedState>,
    PathExtractor(id): PathExtractor<String>,
    Json(body): Json<PatchRepoBody>,
) -> Result<Response, Response> {
    let mut workspace = state.workspace.write().await;
    if workspace.get(&id).is_none() {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            format!("repository '{id}' not found"),
        ));
    }
    if let Some(enabled) = body.enabled {
        workspace.set_enabled(&id, enabled).map_err(bad_request)?;
    }
    if let Some(branches) = body.monitored_branches {
        workspace.set_branches(&id, branches).map_err(bad_request)?;
    }
    workspace.save(&state.config_path).map_err(save_failed)?;
    let repo = workspace.get(&id).cloned().expect("repo checked above");
    Ok(Json(repo).into_response())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DefaultBranchesBody {
    pub default_branches: Vec<String>,
}

pub async fn get_default_branches(State(state): State<SharedState>) -> Json<DefaultBranchesBody> {
    let workspace = state.workspace.read().await;
    Json(DefaultBranchesBody {
        default_branches: workspace.default_branches.clone(),
    })
}

pub async fn put_default_branches(
    State(state): State<SharedState>,
    Json(body): Json<DefaultBranchesBody>,
) -> Result<Response, Response> {
    let mut workspace = state.workspace.write().await;
    let previous = std::mem::replace(
        &mut workspace.default_branches,
        body.default_branches.clone(),
    );
    if let Err(err) = workspace.validate() {
        workspace.default_branches = previous;
        return Err(bad_request(err));
    }
    workspace.save(&state.config_path).map_err(save_failed)?;
    Ok(Json(body).into_response())
}

#[derive(Debug, Serialize)]
pub struct BranchStatus {
    pub branch: String,
    pub last_sha: Option<String>,
    pub current_sha: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RepoStatus {
    pub id: String,
    pub github: String,
    pub enabled: bool,
    pub path: String,
    pub cloned: bool,
    pub branches: Vec<BranchStatus>,
}

pub async fn status(State(state): State<SharedState>) -> Json<Vec<RepoStatus>> {
    let workspace = state.workspace.read().await;
    let index_state = IndexState::load(&state_path(&state.cache_dir)).unwrap_or_default();

    let mut out = Vec::new();
    for repo in &workspace.repositories {
        let path = resolve_repo_path(&state.cache_dir, repo);
        let cloned = path.exists();
        let handle = if cloned {
            RepoHandle::open(&path).ok()
        } else {
            None
        };
        let branches = workspace
            .resolved_branches(repo)
            .into_iter()
            .map(|branch| {
                let last_sha = index_state.last_sha(&repo.id, &branch).map(str::to_string);
                let current_sha = handle
                    .as_ref()
                    .and_then(|handle| branch_sha(handle, &branch).ok())
                    .flatten();
                BranchStatus {
                    branch,
                    last_sha,
                    current_sha,
                }
            })
            .collect();
        out.push(RepoStatus {
            id: repo.id.clone(),
            github: repo.github_or_id().to_string(),
            enabled: repo.enabled,
            path: path.display().to_string(),
            cloned,
            branches,
        });
    }
    Json(out)
}

pub async fn trigger_index(
    State(state): State<SharedState>,
    PathExtractor(id): PathExtractor<String>,
) -> Result<Response, Response> {
    {
        let workspace = state.workspace.read().await;
        if workspace.get(&id).is_none() {
            return Err(error_response(
                StatusCode::NOT_FOUND,
                format!("repository '{id}' not found"),
            ));
        }
    }
    if !try_lock_active(&state, &id) {
        return Err(error_response(
            StatusCode::CONFLICT,
            format!("repository '{id}' is already being indexed"),
        ));
    }
    if state
        .job_tx
        .send(IndexJob {
            repo_id: id.clone(),
            branch: None,
        })
        .is_err()
    {
        release_active(&state, &id);
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "index worker is not running",
        ));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"status": "started"})),
    )
        .into_response())
}

#[derive(Debug, Deserialize)]
pub struct HookRepository {
    pub name: Option<String>,
    pub full_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GitHookPayload {
    pub repo: Option<String>,
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub old: Option<String>,
    pub new: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub repository: Option<HookRepository>,
}

#[derive(Debug, Serialize)]
pub struct HookResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

impl HookResponse {
    fn ignored(reason: &str) -> Self {
        Self {
            status: "ignored".to_string(),
            reason: Some(reason.to_string()),
            repo: None,
            branch: None,
        }
    }
}

pub fn branch_from_ref(git_ref: &str) -> Option<String> {
    if let Some(branch) = git_ref.strip_prefix("refs/heads/") {
        if branch.is_empty() {
            return None;
        }
        return Some(branch.to_string());
    }
    if git_ref.starts_with("refs/") {
        return None;
    }
    if git_ref.is_empty() {
        return None;
    }
    Some(git_ref.to_string())
}

fn hook_accepted(body: HookResponse) -> Response {
    (StatusCode::ACCEPTED, Json(body)).into_response()
}

pub async fn git_hook(
    State(state): State<SharedState>,
    Json(payload): Json<GitHookPayload>,
) -> Result<Response, Response> {
    let workspace = state.workspace.read().await;

    let repo_key = payload
        .repo
        .clone()
        .or_else(|| {
            payload
                .repository
                .as_ref()
                .and_then(|repo| repo.full_name.clone())
        })
        .or_else(|| {
            payload
                .repository
                .as_ref()
                .and_then(|repo| repo.name.clone())
        });
    let Some(repo_key) = repo_key.filter(|key| !key.trim().is_empty()) else {
        return Ok(hook_accepted(HookResponse::ignored(
            "missing repository identifier",
        )));
    };

    let Some(repo) = workspace
        .repositories
        .iter()
        .find(|repo| repo.id == repo_key || repo.github == repo_key)
        .cloned()
    else {
        return Ok(hook_accepted(HookResponse::ignored(
            "repository not configured",
        )));
    };
    if !repo.enabled {
        return Ok(hook_accepted(HookResponse::ignored("repository disabled")));
    }
    let Some(git_ref) = payload.git_ref.as_deref() else {
        return Ok(hook_accepted(HookResponse::ignored("missing ref")));
    };
    let Some(branch) = branch_from_ref(git_ref) else {
        return Ok(hook_accepted(HookResponse::ignored("ref is not a branch")));
    };
    let resolved = workspace.resolved_branches(&repo);
    drop(workspace);
    if !resolved.iter().any(|candidate| candidate == &branch) {
        return Ok(hook_accepted(HookResponse::ignored("branch not monitored")));
    }

    if !try_lock_active(&state, &repo.id) {
        return Ok(hook_accepted(HookResponse {
            status: "already_running".to_string(),
            reason: Some("index already in progress".to_string()),
            repo: Some(repo.id.clone()),
            branch: Some(branch),
        }));
    }
    if state
        .job_tx
        .send(IndexJob {
            repo_id: repo.id.clone(),
            branch: Some(branch.clone()),
        })
        .is_err()
    {
        release_active(&state, &repo.id);
        return Err(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "index worker is not running",
        ));
    }
    Ok(hook_accepted(HookResponse {
        status: "started".to_string(),
        reason: None,
        repo: Some(repo.id.clone()),
        branch: Some(branch),
    }))
}

async fn background_index(state: &SharedState, job: &IndexJob) -> Result<usize> {
    let store = crate::pipeline::connect_store(
        &state.neo4j.uri,
        &state.neo4j.user,
        &state.neo4j.password,
        &state.neo4j.database,
    )?;
    let outcome = run_index_pass(
        &store,
        &state.config_path,
        &state.cache_dir,
        Some(&job.repo_id),
        job.branch.as_deref(),
        false,
    )
    .await?;
    if !outcome.failures.is_empty() {
        anyhow::bail!(
            "{} failure(s): {}",
            outcome.failures.len(),
            outcome.failures.join("; ")
        );
    }
    Ok(outcome.indexed)
}

pub fn router(state: SharedState) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/api/repos", get(list_repos).post(add_repo))
        .route(
            "/api/repos/{id}",
            axum::routing::delete(delete_repo).patch(patch_repo),
        )
        .route("/api/repos/{id}/index", post(trigger_index))
        .route(
            "/api/default-branches",
            get(get_default_branches).put(put_default_branches),
        )
        .route("/api/status", get(status))
        .route("/hooks/git", post(git_hook))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

pub async fn serve(args: ServeArgs) -> Result<()> {
    let workspace = load_or_init_workspace(&args.config)?;
    let neo4j = args.neo4j.settings();
    let (state, mut job_rx) = AppState::new(
        args.config.clone(),
        args.cache_dir.clone(),
        workspace,
        neo4j,
    );

    let worker_state = state.clone();
    tokio::spawn(async move {
        while let Some(job) = job_rx.recv().await {
            info!(repo = %job.repo_id, branch = ?job.branch, "background index started");
            let result = background_index(&worker_state, &job).await;
            release_active(&worker_state, &job.repo_id);
            match result {
                Ok(indexed) => {
                    info!(repo = %job.repo_id, indexed, "background index finished");
                }
                Err(err) => {
                    error!(repo = %job.repo_id, "background index failed: {err:#}");
                }
            }
        }
    });

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    info!(%addr, config = %args.config.display(), "ckg-indexer serve listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;

    struct TestEnv {
        state: SharedState,
        rx: mpsc::UnboundedReceiver<IndexJob>,
        _tmp: tempfile::TempDir,
    }

    fn test_env(workspace: WorkspaceConfig) -> TestEnv {
        let tmp = tempfile::tempdir().expect("tempdir");
        let config_path = tmp.path().join("workspace.toml");
        workspace.save(&config_path).expect("save config");
        let cache_dir = tmp.path().join("cache");
        let (state, rx) =
            AppState::new(config_path, cache_dir, workspace, Neo4jSettings::default());
        TestEnv {
            state,
            rx,
            _tmp: tmp,
        }
    }

    fn sample_repo_json(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "github": format!("acme/{id}"),
            "clone_url": format!("https://example.com/{id}.git"),
            "enabled": true,
            "monitored_branches": ["main"],
        })
    }

    async fn request(
        app: &Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> Response {
        let builder = axum::http::Request::builder().method(method).uri(uri);
        let builder = match &body {
            Some(_) => builder.header("content-type", "application/json"),
            None => builder,
        };
        let req = builder
            .body(match body {
                Some(value) => Body::from(serde_json::to_vec(&value).expect("json body")),
                None => Body::empty(),
            })
            .expect("request");
        app.clone().oneshot(req).await.expect("response")
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body bytes");
        if bytes.is_empty() {
            return serde_json::Value::Null;
        }
        serde_json::from_slice(&bytes).expect("json body")
    }

    #[tokio::test]
    async fn add_get_delete_repo_roundtrip() {
        let env = test_env(WorkspaceConfig::default());
        let app = router(env.state.clone());

        let resp = request(&app, "GET", "/api/repos", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await, serde_json::json!([]));

        let resp = request(
            &app,
            "POST",
            "/api/repos",
            Some(sample_repo_json("payments")),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);

        let resp = request(&app, "GET", "/api/repos", None).await;
        let repos = body_json(resp).await;
        assert_eq!(repos.as_array().unwrap().len(), 1);
        assert_eq!(repos[0]["id"], "payments");

        let reloaded = WorkspaceConfig::load(&env.state.config_path).expect("persisted");
        assert_eq!(reloaded.repositories.len(), 1);

        let resp = request(&app, "DELETE", "/api/repos/payments", None).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let resp = request(&app, "GET", "/api/repos", None).await;
        assert_eq!(body_json(resp).await, serde_json::json!([]));

        let resp = request(&app, "DELETE", "/api/repos/payments", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn add_repo_validation_returns_400() {
        let env = test_env(WorkspaceConfig::default());
        let app = router(env.state.clone());

        let invalid = serde_json::json!({
            "id": "bad",
            "github": "",
            "clone_url": "https://example.com/bad.git",
            "monitored_branches": ["main"],
        });
        let resp = request(&app, "POST", "/api/repos", Some(invalid)).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert!(body["error"].as_str().is_some());

        let resp = request(&app, "GET", "/api/repos", None).await;
        assert_eq!(body_json(resp).await, serde_json::json!([]));
    }

    #[tokio::test]
    async fn patch_repo_updates_enabled_and_branches() {
        let env = test_env(WorkspaceConfig::default());
        let app = router(env.state.clone());
        request(&app, "POST", "/api/repos", Some(sample_repo_json("portal"))).await;

        let resp = request(
            &app,
            "PATCH",
            "/api/repos/portal",
            Some(serde_json::json!({"enabled": false})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["enabled"], false);

        let resp = request(
            &app,
            "PATCH",
            "/api/repos/portal",
            Some(serde_json::json!({"monitored_branches": ["main", "release"]})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let repo = body_json(resp).await;
        assert_eq!(
            repo["monitored_branches"],
            serde_json::json!(["main", "release"])
        );

        let resp = request(
            &app,
            "PATCH",
            "/api/repos/missing",
            Some(serde_json::json!({"enabled": true})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn default_branches_get_put_roundtrip() {
        let env = test_env(WorkspaceConfig::default());
        let app = router(env.state.clone());

        let resp = request(&app, "GET", "/api/default-branches", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            body_json(resp).await,
            serde_json::json!({"default_branches": []})
        );

        let resp = request(
            &app,
            "PUT",
            "/api/default-branches",
            Some(serde_json::json!({"default_branches": ["main", "develop"]})),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = request(&app, "GET", "/api/default-branches", None).await;
        assert_eq!(
            body_json(resp).await,
            serde_json::json!({"default_branches": ["main", "develop"]})
        );

        let reloaded = WorkspaceConfig::load(&env.state.config_path).expect("persisted");
        assert_eq!(reloaded.default_branches, vec!["main", "develop"]);
    }

    #[tokio::test]
    async fn static_index_served_at_root() {
        let env = test_env(WorkspaceConfig::default());
        let app = router(env.state.clone());
        let resp = request(&app, "GET", "/", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(html.contains("<html"));
        assert!(html.contains("/api/repos"));
    }

    #[tokio::test]
    async fn trigger_index_enqueues_and_blocks_double_index() {
        let mut env = test_env(WorkspaceConfig::default());
        let app = router(env.state.clone());
        request(
            &app,
            "POST",
            "/api/repos",
            Some(sample_repo_json("payments")),
        )
        .await;

        let resp = request(&app, "POST", "/api/repos/payments/index", None).await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        assert_eq!(
            body_json(resp).await,
            serde_json::json!({"status": "started"})
        );

        let job = env.rx.try_recv().expect("job enqueued");
        assert_eq!(job.repo_id, "payments");
        assert_eq!(job.branch, None);

        let resp = request(&app, "POST", "/api/repos/payments/index", None).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT);

        let resp = request(&app, "POST", "/api/repos/missing/index", None).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn git_hook_enqueues_index_for_monitored_branch() {
        let mut workspace = WorkspaceConfig::default();
        workspace.default_branches = vec!["main".to_string()];
        workspace
            .add_repo(sample_workspace_repo("payments"))
            .expect("add repo");
        let mut env = test_env(workspace);
        let app = router(env.state.clone());

        let resp = request(
            &app,
            "POST",
            "/hooks/git",
            Some(serde_json::json!({
                "repo": "payments",
                "ref": "refs/heads/main",
                "old": "aaaa",
                "new": "bbbb",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "started");
        assert_eq!(body["repo"], "payments");
        assert_eq!(body["branch"], "main");

        let job = env.rx.try_recv().expect("hook job enqueued");
        assert_eq!(job.repo_id, "payments");
        assert_eq!(job.branch.as_deref(), Some("main"));

        let resp = request(
            &app,
            "POST",
            "/hooks/git",
            Some(serde_json::json!({
                "repo": "payments",
                "ref": "refs/heads/unmonitored",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "ignored");
        assert!(env.rx.try_recv().is_err(), "no job for unmonitored branch");

        let resp = request(
            &app,
            "POST",
            "/hooks/git",
            Some(serde_json::json!({
                "repository": { "full_name": "acme/payments" },
                "ref": "refs/heads/main",
                "before": "aaaa",
                "after": "bbbb",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "already_running", "double guard");
    }

    #[tokio::test]
    async fn git_hook_triggers_repo_level_monitored_branch() {
        let mut workspace = WorkspaceConfig::default();
        workspace.default_branches = vec!["main".to_string()];
        let mut repo = sample_workspace_repo("svc");
        repo.monitored_branches = vec!["develop".to_string(), "release".to_string()];
        workspace.add_repo(repo).expect("add repo");
        let mut env = test_env(workspace);
        let app = router(env.state.clone());

        let resp = request(
            &app,
            "POST",
            "/hooks/git",
            Some(serde_json::json!({
                "repo": "svc",
                "ref": "refs/heads/develop",
                "old": "aaaa",
                "new": "bbbb",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "started");
        assert_eq!(body["repo"], "svc");
        assert_eq!(body["branch"], "develop");

        let job = env.rx.try_recv().expect("hook job enqueued");
        assert_eq!(job.repo_id, "svc");
        assert_eq!(job.branch.as_deref(), Some("develop"));

        let resp = request(
            &app,
            "POST",
            "/hooks/git",
            Some(serde_json::json!({
                "repo": "svc",
                "ref": "refs/heads/main",
            })),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
        let body = body_json(resp).await;
        assert_eq!(body["status"], "ignored");
        assert_eq!(body["reason"], "branch not monitored");
        assert!(
            env.rx.try_recv().is_err(),
            "workspace default branch must not bypass repo monitored_branches"
        );
    }

    #[tokio::test]
    async fn status_reports_enabled_repos() {
        let mut workspace = WorkspaceConfig::default();
        workspace
            .add_repo(sample_workspace_repo("payments"))
            .expect("add repo");
        let env = test_env(workspace);
        let app = router(env.state.clone());

        let resp = request(&app, "GET", "/api/status", None).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let repos = body.as_array().expect("array");
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0]["id"], "payments");
        assert_eq!(repos[0]["enabled"], true);
        assert_eq!(repos[0]["cloned"], false);
        let branches = repos[0]["branches"].as_array().unwrap();
        assert_eq!(branches[0]["branch"], "main");
        assert_eq!(branches[0]["last_sha"], serde_json::Value::Null);
    }

    #[test]
    fn branch_from_ref_maps_heads_and_rejects_tags() {
        assert_eq!(branch_from_ref("refs/heads/main").as_deref(), Some("main"));
        assert_eq!(branch_from_ref("main").as_deref(), Some("main"));
        assert_eq!(branch_from_ref("refs/tags/v1"), None);
        assert_eq!(branch_from_ref(""), None);
        assert_eq!(branch_from_ref("refs/heads/"), None);
    }

    #[test]
    fn load_or_init_creates_missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/workspace.toml");
        assert!(!path.exists());
        let workspace = load_or_init_workspace(&path).expect("init");
        assert!(workspace.repositories.is_empty());
        assert!(path.exists());
        let reloaded = WorkspaceConfig::load(&path).expect("reload");
        assert_eq!(reloaded, workspace);
    }

    fn sample_workspace_repo(id: &str) -> RepositoryConfig {
        RepositoryConfig::new(
            id,
            format!("acme/{id}"),
            format!("https://example.com/{id}.git"),
        )
        .with_branches(["main"])
    }
}
