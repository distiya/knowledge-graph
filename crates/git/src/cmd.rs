use std::path::Path;
use std::process::{Command, Output};

use crate::Result;
use crate::error::GitError;

pub(crate) fn git_output(cwd: Option<&Path>, args: &[&str]) -> Result<Output> {
    let mut cmd = Command::new("git");
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    Ok(cmd.output()?)
}

pub(crate) fn run_git(cwd: Option<&Path>, args: &[&str]) -> Result<String> {
    let out = git_output(cwd, args)?;
    if !out.status.success() {
        return Err(GitError::CommandFailed {
            command: format!("git {}", args.join(" ")),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
