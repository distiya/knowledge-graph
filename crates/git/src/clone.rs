use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;
use crate::cmd::run_git;
use crate::error::GitError;
use crate::repo::RepoHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneConfig {
    pub url: String,
    pub cache_dir: PathBuf,
    pub branch: Option<String>,
}

impl CloneConfig {
    pub fn new(url: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            url: url.into(),
            cache_dir: cache_dir.into(),
            branch: None,
        }
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    pub fn repo_path(&self) -> PathBuf {
        self.cache_dir.join(repo_name_from_url(&self.url))
    }
}

pub fn repo_name_from_url(url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    let after_scheme = match trimmed.find("://") {
        Some(idx) => &trimmed[idx + 3..],
        None => trimmed,
    };
    let last = after_scheme.rsplit('/').next().unwrap_or_default();
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() || name == "." || name == ".." {
        "repo".to_string()
    } else {
        name.to_string()
    }
}

pub fn clone_or_open(config: &CloneConfig) -> Result<RepoHandle> {
    let path = config.repo_path();

    if gix::open(&path).is_ok() {
        let handle = RepoHandle::open(&path)?;
        fetch_updates(&handle)?;
        return Ok(handle);
    }

    if path.exists() {
        let empty = path.is_dir() && fs::read_dir(&path)?.next().is_none();
        if !empty {
            return Err(GitError::TargetOccupied(path));
        }
    }

    fs::create_dir_all(&config.cache_dir)?;
    clone_repo(&config.url, &path, config.branch.as_deref())?;
    RepoHandle::open(&path)
}

pub fn open_and_fetch(config: &CloneConfig) -> Result<RepoHandle> {
    let handle = RepoHandle::open(config.repo_path())?;
    fetch_updates(&handle)?;
    Ok(handle)
}

fn is_local_url(url: &str) -> bool {
    if let Some((scheme, _)) = url.split_once("://") {
        return scheme.eq_ignore_ascii_case("file");
    }
    !(url.contains('@') && url.contains(':'))
}

pub fn clone_repo(url: &str, dest: &Path, branch: Option<&str>) -> Result<()> {
    let dest_str = dest.to_string_lossy().into_owned();
    let allow_filter = !is_local_url(url);

    let attempt = |use_filter: bool| -> Result<std::process::Output> {
        let mut cmd = Command::new("git");
        cmd.arg("clone");
        if use_filter {
            cmd.arg("--filter=blob:none");
        }
        if let Some(branch) = branch {
            cmd.arg("-b").arg(branch);
        }
        cmd.arg(url).arg(&dest_str);
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        Ok(cmd.output()?)
    };

    let filtered = if allow_filter {
        Some(attempt(true)?)
    } else {
        None
    };
    if let Some(out) = &filtered {
        if out.status.success() {
            return Ok(());
        }
    }

    if dest.is_dir() {
        fs::remove_dir_all(dest)?;
    }

    let plain = attempt(false)?;
    if plain.status.success() {
        return Ok(());
    }

    let filter_detail = match &filtered {
        Some(out) => format!(
            "with --filter: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        None => "with --filter: skipped (local url)".to_string(),
    };
    Err(GitError::CommandFailed {
        command: format!("git clone {url} {}", dest.display()),
        stderr: format!(
            "{filter_detail}; without --filter: {}",
            String::from_utf8_lossy(&plain.stderr).trim()
        ),
    })
}

pub fn fetch_updates(handle: &RepoHandle) -> Result<()> {
    let has_origin = run_git(Some(handle.path()), &["remote", "get-url", "origin"]).is_ok();
    if has_origin {
        run_git(Some(handle.path()), &["fetch", "--prune", "origin"])?;
    }
    Ok(())
}
