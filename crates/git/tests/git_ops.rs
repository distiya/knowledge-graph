use std::fs;
use std::path::Path;
use std::process::Command;

use ckg_git::{CloneConfig, RepoHandle, clone_or_open, detect_language, repo_name_from_url};

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

fn init_repo(dir: &Path) {
    git(dir, &["init", "-q"]);
}

fn commit_all(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn head_branch_and_listing_shas() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::write(dir.join("lib.rs"), "pub fn x() {}\n").unwrap();
    let sha = commit_all(dir, "initial");

    let repo = RepoHandle::open(dir).unwrap();
    assert_eq!(repo.path(), dir);
    assert_eq!(repo.head_commit_sha().unwrap(), sha);
    assert_eq!(sha.len(), 40);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));

    git(dir, &["branch", "feature"]);
    assert_eq!(
        repo.branch_commit_sha("feature").unwrap().as_deref(),
        Some(sha.as_str())
    );
    assert_eq!(repo.branch_commit_sha("does-not-exist").unwrap(), None);

    let branches = repo.list_branches().unwrap();
    assert!(branches.contains(&"feature".to_string()));
    assert_eq!(branches.iter().filter(|b| *b == "feature").count(), 1);
    assert!(branches.iter().any(|b| b == "main" || b == "master"));
}

#[test]
fn file_at_returns_content_at_revision() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    let content = "fn main() {\n    println!(\"hi\");\n}\n";
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/main.rs"), content).unwrap();
    let sha = commit_all(dir, "add main");

    let repo = RepoHandle::open(dir).unwrap();
    assert_eq!(
        repo.file_at(&sha, "src/main.rs").unwrap().as_deref(),
        Some(content)
    );
    assert_eq!(repo.file_at(&sha, "missing.rs").unwrap(), None);
    assert_eq!(repo.file_at(&sha, "src").unwrap(), None);
    assert_eq!(repo.file_at("not-a-sha", "src/main.rs").unwrap(), None);

    fs::write(dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    let sha2 = commit_all(dir, "rewrite main");
    assert_eq!(
        repo.file_at(&sha, "src/main.rs").unwrap().as_deref(),
        Some(content)
    );
    assert_eq!(
        repo.file_at(&sha2, "src/main.rs").unwrap().as_deref(),
        Some("fn main() {}\n")
    );
}

#[test]
fn diff_files_categorizes_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::write(dir.join("a.txt"), "a\n").unwrap();
    fs::write(dir.join("b.txt"), "b\n").unwrap();
    fs::write(dir.join("c.txt"), "c\n").unwrap();
    let old_sha = commit_all(dir, "first");

    fs::write(dir.join("a.txt"), "a2\n").unwrap();
    fs::remove_file(dir.join("b.txt")).unwrap();
    fs::write(dir.join("d.txt"), "d\n").unwrap();
    git(dir, &["mv", "c.txt", "e.txt"]);
    let new_sha = commit_all(dir, "second");

    let repo = RepoHandle::open(dir).unwrap();
    let diff = repo.diff_files(&old_sha, &new_sha).unwrap();
    assert_eq!(diff.added, vec!["d.txt".to_string()]);
    assert_eq!(diff.modified, vec!["a.txt".to_string()]);
    assert_eq!(diff.deleted, vec!["b.txt".to_string()]);
    assert_eq!(
        diff.renamed,
        vec![("c.txt".to_string(), "e.txt".to_string())]
    );

    let empty = repo.diff_files(&old_sha, &old_sha).unwrap();
    assert!(empty.added.is_empty());
    assert!(empty.modified.is_empty());
    assert!(empty.deleted.is_empty());
    assert!(empty.renamed.is_empty());
}

#[test]
fn list_files_at_skips_build_dirs_and_binaries() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    init_repo(dir);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("target/debug")).unwrap();
    fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(dir.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
    fs::write(dir.join("README.md"), "# readme\n").unwrap();
    fs::write(dir.join("target/debug/out.rs"), "fn generated() {}\n").unwrap();
    fs::write(
        dir.join("node_modules/pkg/index.js"),
        "module.exports = 1;\n",
    )
    .unwrap();
    fs::write(
        dir.join("assets/logo.png"),
        [0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A],
    )
    .unwrap();
    fs::write(dir.join("tools.py"), "print('hi')\n").unwrap();
    let sha = commit_all(dir, "files");

    let repo = RepoHandle::open(dir).unwrap();
    let files = repo.list_files_at(&sha).unwrap();
    assert_eq!(
        files,
        vec![
            "README.md".to_string(),
            "src/lib.rs".to_string(),
            "tools.py".to_string(),
        ]
    );
}

#[test]
fn detect_language_maps_extensions() {
    assert_eq!(detect_language("src/main.rs"), Some("rust"));
    assert_eq!(detect_language("app.js"), Some("javascript"));
    assert_eq!(detect_language("app.mjs"), Some("javascript"));
    assert_eq!(detect_language("app.cjs"), Some("javascript"));
    assert_eq!(detect_language("app.jsx"), Some("javascript"));
    assert_eq!(detect_language("mod.ts"), Some("typescript"));
    assert_eq!(detect_language("view.tsx"), Some("typescript"));
    assert_eq!(detect_language("script.py"), Some("python"));
    assert_eq!(detect_language("script.pyi"), Some("python"));
    assert_eq!(detect_language("main.go"), Some("go"));
    assert_eq!(detect_language("README.md"), Some("markdown"));
    assert_eq!(detect_language("Makefile"), Some("make"));
    assert_eq!(detect_language("lib"), None);
}

#[test]
fn detect_language_covers_must_have_and_common_extensions() {
    assert_eq!(detect_language("Main.java"), Some("java"));
    assert_eq!(detect_language("src/util.c"), Some("c"));
    assert_eq!(detect_language("src/core.cpp"), Some("cpp"));
    assert_eq!(detect_language("src/core.cc"), Some("cpp"));
    assert_eq!(detect_language("App.kt"), Some("kotlin"));
    assert_eq!(detect_language("build.kts"), Some("kotlin"));
    assert_eq!(detect_language("query.sql"), Some("sql"));
    assert_eq!(detect_language("run.sh"), Some("shell"));
    assert_eq!(detect_language("app.rb"), Some("ruby"));
    assert_eq!(detect_language("index.php"), Some("php"));
    assert_eq!(detect_language("Program.cs"), Some("csharp"));
    assert_eq!(detect_language("Main.scala"), Some("scala"));
    assert_eq!(detect_language("main.dart"), Some("dart"));
    assert_eq!(detect_language("lib.lua"), Some("lua"));
    assert_eq!(detect_language("plot.R"), Some("r"));
    assert_eq!(detect_language("main.jl"), Some("julia"));
    assert_eq!(detect_language("script.pl"), Some("perl"));
    assert_eq!(detect_language("app.ex"), Some("elixir"));
    assert_eq!(detect_language("app.exs"), Some("elixir"));
    assert_eq!(detect_language("Main.hs"), Some("haskell"));
    assert_eq!(detect_language("util.ml"), Some("ocaml"));
    assert_eq!(detect_language("src/main.zig"), Some("zig"));
    assert_eq!(detect_language("index.html"), Some("html"));
    assert_eq!(detect_language("style.css"), Some("css"));
    assert_eq!(detect_language("style.scss"), Some("scss"));
    assert_eq!(detect_language("data.xml"), Some("xml"));
    assert_eq!(detect_language("config.yml"), Some("yaml"));
    assert_eq!(detect_language("config.yaml"), Some("yaml"));
    assert_eq!(detect_language("data.json"), Some("json"));
    assert_eq!(detect_language("docs.md"), Some("markdown"));
    assert_eq!(detect_language("Cargo.toml"), Some("toml"));
    assert_eq!(detect_language("api.proto"), Some("proto"));
    assert_eq!(detect_language("main.tf"), Some("terraform"));
    assert_eq!(detect_language("net.hcl"), Some("hcl"));
    assert_eq!(detect_language("script.ps1"), Some("powershell"));
    assert_eq!(detect_language("run.bat"), Some("batch"));
    assert_eq!(detect_language("build.cmake"), Some("cmake"));
    assert_eq!(detect_language("CMakeLists.txt"), Some("cmake"));
    assert_eq!(detect_language("Dockerfile"), Some("dockerfile"));
}

#[test]
fn clone_or_open_clones_then_reopens_and_fetches() {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin");
    fs::create_dir_all(&origin).unwrap();
    init_repo(&origin);
    fs::write(origin.join("hello.rs"), "fn hello() {}\n").unwrap();
    let origin_sha = commit_all(&origin, "first");
    let default_branch = git(&origin, &["rev-parse", "--abbrev-ref", "HEAD"]);

    let cache = tmp.path().join("cache");
    let config = CloneConfig::new(origin.to_string_lossy(), cache.as_path());
    assert_eq!(config.repo_path(), cache.join("origin"));

    let repo = clone_or_open(&config).unwrap();
    assert_eq!(repo.path(), cache.join("origin"));
    assert!(repo.path().join(".git").exists());
    assert_eq!(repo.head_commit_sha().unwrap(), origin_sha);
    assert!(
        repo.list_branches()
            .unwrap()
            .contains(&default_branch.clone())
    );

    fs::write(origin.join("world.rs"), "fn world() {}\n").unwrap();
    let new_sha = commit_all(&origin, "second");

    let repo = clone_or_open(&config).unwrap();
    assert_eq!(repo.head_commit_sha().unwrap(), origin_sha);
    assert_eq!(
        repo.branch_commit_sha(&default_branch).unwrap().as_deref(),
        Some(origin_sha.as_str())
    );
    let remote_branch = format!("origin/{default_branch}");
    assert_eq!(
        repo.branch_commit_sha(&remote_branch).unwrap().as_deref(),
        Some(new_sha.as_str())
    );
}

#[test]
fn clone_or_open_rejects_occupied_target() {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin");
    fs::create_dir_all(&origin).unwrap();
    init_repo(&origin);
    fs::write(origin.join("a.txt"), "a\n").unwrap();
    commit_all(&origin, "first");

    let cache = tmp.path().join("cache");
    fs::create_dir_all(cache.join("origin")).unwrap();
    fs::write(cache.join("origin/junk.txt"), "junk\n").unwrap();

    let config = CloneConfig::new(origin.to_string_lossy(), cache.as_path());
    assert!(clone_or_open(&config).is_err());
}

#[test]
fn open_rejects_non_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let err = RepoHandle::open(tmp.path()).unwrap_err();
    assert!(matches!(err, ckg_git::GitError::NotARepository(_)));
}

#[test]
fn repo_name_derived_from_urls() {
    assert_eq!(
        repo_name_from_url("https://github.com/acme/widget.git"),
        "widget"
    );
    assert_eq!(
        repo_name_from_url("https://github.com/acme/widget"),
        "widget"
    );
    assert_eq!(
        repo_name_from_url("git@github.com:acme/widget.git"),
        "widget"
    );
    assert_eq!(repo_name_from_url("file:///srv/repos/widget.git"), "widget");
    assert_eq!(repo_name_from_url("/srv/repos/widget"), "widget");
    assert_eq!(
        repo_name_from_url("https://github.com/acme/widget/"),
        "widget"
    );
}
