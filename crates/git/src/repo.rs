use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::cmd::run_git;
use crate::error::GitError;

const SKIPPED_DIRS: &[&str] = &["target", "node_modules", ".git"];

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "ico", "icns", "tif", "tiff", "pdf", "zip", "gz",
    "tgz", "bz2", "xz", "zst", "7z", "rar", "jar", "war", "exe", "dll", "so", "dylib", "a", "o",
    "obj", "lib", "bin", "class", "pyc", "pyo", "wasm", "db", "sqlite", "sqlite3", "woff", "woff2",
    "ttf", "otf", "eot", "mp3", "mp4", "ogg", "wav", "flac", "avi", "mov", "webm", "mkv", "whl",
    "deb", "rpm", "dmg", "iso", "img", "parquet", "avro", "lockb",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub modified: Vec<String>,
    #[serde(default)]
    pub deleted: Vec<String>,
    #[serde(default)]
    pub renamed: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct RepoHandle {
    path: PathBuf,
}

impl RepoHandle {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        gix::open(&path).map_err(|_| GitError::NotARepository(path.clone()))?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn repository(&self) -> Result<gix::Repository> {
        gix::open(&self.path).map_err(|e| GitError::Other(e.to_string()))
    }

    pub fn head_commit_sha(&self) -> Result<String> {
        let out = run_git(
            Some(&self.path),
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )?;
        Ok(out.trim().to_string())
    }

    pub fn branch_commit_sha(&self, branch: &str) -> Result<Option<String>> {
        for spec in ref_candidates(branch) {
            match run_git(
                Some(&self.path),
                &["rev-parse", "--verify", "--quiet", &spec],
            ) {
                Ok(out) => return Ok(Some(out.trim().to_string())),
                Err(GitError::CommandFailed { .. }) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    pub fn list_branches(&self) -> Result<Vec<String>> {
        let out = run_git(
            Some(&self.path),
            &[
                "for-each-ref",
                "--format=%(refname:short)",
                "refs/heads",
                "refs/remotes",
            ],
        )?;
        let mut branches: Vec<String> = out
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.ends_with("/HEAD"))
            .map(String::from)
            .collect();
        branches.sort();
        branches.dedup();
        Ok(branches)
    }

    pub fn file_at(&self, commit_sha: &str, path: &str) -> Result<Option<String>> {
        let spec = format!("{commit_sha}:{path}");
        let kind = match run_git(Some(&self.path), &["cat-file", "-t", &spec]) {
            Ok(kind) => kind,
            Err(GitError::CommandFailed { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        if kind.trim() != "blob" {
            return Ok(None);
        }
        let content = run_git(Some(&self.path), &["cat-file", "blob", &spec])?;
        Ok(Some(content))
    }

    pub fn diff_files(&self, old_sha: &str, new_sha: &str) -> Result<FileDiff> {
        let out = run_git(
            Some(&self.path),
            &["diff", "--name-status", "-M", "-z", old_sha, new_sha],
        )?;
        let mut diff = FileDiff::default();
        let mut parts = out.split('\0').filter(|part| !part.is_empty());
        while let Some(status) = parts.next() {
            match status.chars().next() {
                Some('A') => {
                    if let Some(path) = parts.next() {
                        diff.added.push(path.to_string());
                    }
                }
                Some('M') | Some('T') => {
                    if let Some(path) = parts.next() {
                        diff.modified.push(path.to_string());
                    }
                }
                Some('D') => {
                    if let Some(path) = parts.next() {
                        diff.deleted.push(path.to_string());
                    }
                }
                Some('R') | Some('C') => {
                    let old = parts.next().map(str::to_string);
                    let new = parts.next().map(str::to_string);
                    if let (Some(old), Some(new)) = (old, new) {
                        diff.renamed.push((old, new));
                    }
                }
                _ => {}
            }
        }
        diff.added.sort();
        diff.modified.sort();
        diff.deleted.sort();
        diff.renamed.sort();
        Ok(diff)
    }

    pub fn list_files_at(&self, commit_sha: &str) -> Result<Vec<String>> {
        let out = run_git(
            Some(&self.path),
            &["ls-tree", "-r", "--name-only", "-z", commit_sha],
        )?;
        Ok(out
            .split('\0')
            .filter(|path| !path.is_empty())
            .filter(|path| {
                !path
                    .split('/')
                    .any(|component| SKIPPED_DIRS.contains(&component))
            })
            .filter(|path| !is_binary_path(path))
            .map(str::to_string)
            .collect())
    }
}

fn ref_candidates(branch: &str) -> Vec<String> {
    if let Some(rest) = branch.strip_prefix("refs/") {
        return vec![format!("refs/{rest}^{{commit}}")];
    }
    vec![
        format!("refs/heads/{branch}^{{commit}}"),
        format!("refs/remotes/{branch}^{{commit}}"),
        format!("refs/remotes/origin/{branch}^{{commit}}"),
    ]
}

fn is_binary_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| BINARY_EXTENSIONS.contains(&ext.as_str()))
}
