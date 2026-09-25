use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use ckg_domain::CanonicalId;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::error::ConfigError;

pub fn default_analyzers() -> Vec<String> {
    vec!["tree-sitter".to_string()]
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingConfig {
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default = "default_analyzers")]
    pub analyzers: Vec<String>,
    #[serde(default)]
    pub scip_enabled: bool,
    #[serde(default)]
    pub joern_enabled: bool,
}

impl Default for IndexingConfig {
    fn default() -> Self {
        Self {
            languages: Vec::new(),
            analyzers: default_analyzers(),
            scip_enabled: false,
            joern_enabled: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryConfig {
    pub id: String,
    pub github: String,
    pub clone_url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub monitored_branches: Vec<String>,
    #[serde(default)]
    pub indexing: IndexingConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
}

impl RepositoryConfig {
    pub fn new(
        id: impl Into<String>,
        github: impl Into<String>,
        clone_url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            github: github.into(),
            clone_url: clone_url.into(),
            enabled: true,
            monitored_branches: vec!["main".to_string()],
            indexing: IndexingConfig::default(),
            local_path: None,
        }
    }

    pub fn with_branches<I, S>(mut self, branches: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.monitored_branches = branches.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_indexing(mut self, indexing: IndexingConfig) -> Self {
        self.indexing = indexing;
        self
    }

    pub fn with_local_path(mut self, local_path: impl Into<String>) -> Self {
        self.local_path = Some(local_path.into());
        self
    }

    pub fn github_or_id(&self) -> &str {
        if self.github.trim().is_empty() {
            &self.id
        } else {
            &self.github
        }
    }

    pub fn canonical_id(&self) -> CanonicalId {
        CanonicalId::from_parts(&["repository", self.github_or_id()])
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_with_default_branches(&[])
    }

    pub fn validate_with_default_branches(&self, default_branches: &[String]) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(ConfigError::Validation(format!(
                "repository id must not be empty (github '{}')",
                self.github
            )));
        }
        if let Some(local_path) = &self.local_path {
            if local_path.trim().is_empty() {
                return Err(ConfigError::Validation(format!(
                    "repository '{}' local_path must not be empty",
                    self.id
                )));
            }
        }
        if self.github.trim().is_empty() && self.local_path.is_none() {
            return Err(ConfigError::EmptyGithub {
                id: self.id.clone(),
            });
        }
        if self.enabled
            && !self
                .monitored_branches
                .iter()
                .any(|branch| !branch.trim().is_empty())
            && !default_branches
                .iter()
                .any(|branch| !branch.trim().is_empty())
        {
            return Err(ConfigError::NoBranches {
                id: self.id.clone(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinksConfig {
    /// Enable the cross-repository service-dependency link pass.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Base URL / host prefix → repository id (e.g. "https://api.orders.internal" = "orders-api").
    #[serde(default)]
    pub base_urls: BTreeMap<String, String>,
    /// Channel name → broker hint (e.g. "orders.created" = "kafka").
    #[serde(default)]
    pub channel_brokers: BTreeMap<String, String>,
    /// Resource alias/URL → canonical resource target. The value may be a
    /// plain identity or a `"<resource_type>:<identity>"` override (e.g.
    /// "s3://orders-dev/x" = "bucket:orders-prod").
    #[serde(default)]
    pub resources: BTreeMap<String, String>,
}

impl Default for LinksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_urls: BTreeMap::new(),
            channel_brokers: BTreeMap::new(),
            resources: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub repositories: Vec<RepositoryConfig>,
    #[serde(default)]
    pub default_branches: Vec<String>,
    #[serde(default)]
    pub links: LinksConfig,
}

fn has_non_empty_branch(branches: &[String]) -> bool {
    branches.iter().any(|branch| !branch.trim().is_empty())
}

impl WorkspaceConfig {
    pub fn resolved_branches(&self, repo: &RepositoryConfig) -> Vec<String> {
        if has_non_empty_branch(&repo.monitored_branches) {
            return repo.monitored_branches.clone();
        }
        if has_non_empty_branch(&self.default_branches) {
            return self.default_branches.clone();
        }
        vec!["main".to_string()]
    }
}

impl WorkspaceConfig {
    pub fn from_toml(raw: &str) -> Result<Self> {
        let config: Self = toml::from_str(raw)?;
        config.validate()?;
        Ok(config)
    }

    pub fn from_json(raw: &str) -> Result<Self> {
        let config: Self = serde_json::from_str(raw)?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string(self)?)
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)?;
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => Self::from_toml(&raw),
            Some("json") => Self::from_json(&raw),
            other => Err(ConfigError::UnsupportedFormat(
                other.unwrap_or("<none>").to_string(),
            )),
        }
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let raw = match path.extension().and_then(|ext| ext.to_str()) {
            Some("toml") => self.to_toml()?,
            Some("json") => self.to_json()?,
            other => {
                return Err(ConfigError::UnsupportedFormat(
                    other.unwrap_or("<none>").to_string(),
                ));
            }
        };
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        let mut seen = HashSet::new();
        for repo in &self.repositories {
            if !seen.insert(repo.id.as_str()) {
                return Err(ConfigError::DuplicateId(repo.id.clone()));
            }
            repo.validate_with_default_branches(&self.default_branches)?;
        }
        Ok(())
    }

    pub fn add_repo(&mut self, repo: RepositoryConfig) -> Result<()> {
        if self.repositories.iter().any(|r| r.id == repo.id) {
            return Err(ConfigError::DuplicateId(repo.id.clone()));
        }
        repo.validate_with_default_branches(&self.default_branches)?;
        self.repositories.push(repo);
        Ok(())
    }

    pub fn remove_repo(&mut self, id: &str) -> Option<RepositoryConfig> {
        let index = self.repositories.iter().position(|r| r.id == id)?;
        Some(self.repositories.remove(index))
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        let repo = self
            .repositories
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| ConfigError::NotFound(id.to_string()))?;
        let previous = repo.enabled;
        repo.enabled = enabled;
        if let Err(err) = repo.validate_with_default_branches(&self.default_branches) {
            repo.enabled = previous;
            return Err(err);
        }
        Ok(())
    }

    pub fn set_branches<I, S>(&mut self, id: &str, branches: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let branches: Vec<String> = branches.into_iter().map(Into::into).collect();
        let repo = self
            .repositories
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| ConfigError::NotFound(id.to_string()))?;
        let previous = std::mem::replace(&mut repo.monitored_branches, branches);
        if let Err(err) = repo.validate_with_default_branches(&self.default_branches) {
            repo.monitored_branches = previous;
            return Err(err);
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&RepositoryConfig> {
        self.repositories.iter().find(|r| r.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut RepositoryConfig> {
        self.repositories.iter_mut().find(|r| r.id == id)
    }

    pub fn enabled_repos(&self) -> Vec<&RepositoryConfig> {
        self.repositories.iter().filter(|r| r.enabled).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_repo(id: &str) -> RepositoryConfig {
        RepositoryConfig::new(
            id,
            format!("acme/{id}"),
            format!("https://github.com/acme/{id}.git"),
        )
    }

    fn sample_workspace() -> WorkspaceConfig {
        WorkspaceConfig {
            repositories: vec![
                sample_repo("payments").with_branches(["main", "develop"]),
                sample_repo("portal").with_branches(["main"]),
                sample_repo("legacy")
                    .with_enabled(false)
                    .with_branches(["master"]),
            ],
            default_branches: vec!["main".to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn defaults_match_spec() {
        let indexing = IndexingConfig::default();
        assert_eq!(indexing.analyzers, default_analyzers());
        assert_eq!(indexing.analyzers, vec!["tree-sitter".to_string()]);
        assert!(indexing.languages.is_empty());
        assert!(!indexing.scip_enabled);
        assert!(!indexing.joern_enabled);

        let repo = RepositoryConfig::new("id", "org/repo", "https://github.com/org/repo.git");
        assert!(repo.enabled);
        assert_eq!(repo.monitored_branches, vec!["main".to_string()]);
        assert!(repo.validate().is_ok());
    }

    #[test]
    fn canonical_id_is_stable() {
        let repo = sample_repo("payments");
        assert_eq!(repo.canonical_id(), repo.canonical_id());
        assert_ne!(repo.canonical_id(), sample_repo("portal").canonical_id());
    }

    #[test]
    fn toml_roundtrip() {
        let ws = sample_workspace();
        let raw = ws.to_toml().unwrap();
        let back = WorkspaceConfig::from_toml(&raw).unwrap();
        assert_eq!(ws, back);
    }

    #[test]
    fn json_roundtrip() {
        let ws = sample_workspace();
        let raw = ws.to_json().unwrap();
        let back = WorkspaceConfig::from_json(&raw).unwrap();
        assert_eq!(ws, back);
    }

    #[test]
    fn save_and_load_toml_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workspace.toml");
        let ws = sample_workspace();
        ws.save(&path).unwrap();
        let loaded = WorkspaceConfig::load(&path).unwrap();
        assert_eq!(ws, loaded);
    }

    #[test]
    fn save_and_load_json_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested/workspace.json");
        let ws = sample_workspace();
        ws.save(&path).unwrap();
        let loaded = WorkspaceConfig::load(&path).unwrap();
        assert_eq!(ws, loaded);
    }

    #[test]
    fn load_rejects_unknown_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("workspace.yaml");
        std::fs::write(&path, "repositories: []").unwrap();
        let err = WorkspaceConfig::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::UnsupportedFormat(_)));
    }

    #[test]
    fn load_rejects_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let err = WorkspaceConfig::load(tmp.path().join("nope.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::Io(_)));
    }

    #[test]
    fn load_validates_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.toml");
        std::fs::write(
            &path,
            r#"
                [[repositories]]
                id = "broken"
                github = ""
                clone_url = "https://github.com/acme/broken.git"
                enabled = false
            "#,
        )
        .unwrap();
        let err = WorkspaceConfig::load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::EmptyGithub { .. }));
    }

    #[test]
    fn minimal_toml_uses_defaults() {
        let raw = r#"
            [[repositories]]
            id = "api"
            github = "acme/api"
            clone_url = "https://github.com/acme/api.git"
            monitored_branches = ["main"]
        "#;
        let ws = WorkspaceConfig::from_toml(raw).unwrap();
        let repo = ws.get("api").unwrap();
        assert!(repo.enabled);
        assert_eq!(repo.indexing.analyzers, default_analyzers());
        assert!(repo.indexing.languages.is_empty());
        assert!(!repo.indexing.scip_enabled);
        assert!(!repo.indexing.joern_enabled);
    }

    #[test]
    fn example_config_parses_and_validates() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/workspace.example.toml"
        );
        let ws = WorkspaceConfig::load(path).unwrap();
        assert_eq!(ws.repositories.len(), 3);
        ws.validate().unwrap();
    }

    #[test]
    fn add_repo_rejects_duplicates() {
        let mut ws = WorkspaceConfig::default();
        ws.add_repo(sample_repo("api")).unwrap();
        let err = ws.add_repo(sample_repo("api")).unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateId(id) if id == "api"));
        assert_eq!(ws.repositories.len(), 1);
    }

    #[test]
    fn add_repo_rejects_invalid_repo() {
        let mut ws = WorkspaceConfig::default();
        let err = ws
            .add_repo(RepositoryConfig::new(
                "bad",
                "",
                "https://example.com/r.git",
            ))
            .unwrap_err();
        assert!(matches!(err, ConfigError::EmptyGithub { .. }));

        let err = ws
            .add_repo(
                RepositoryConfig::new("bad", "acme/bad", "https://example.com/r.git")
                    .with_branches(Vec::<String>::new()),
            )
            .unwrap_err();
        assert!(matches!(err, ConfigError::NoBranches { .. }));
        assert!(ws.repositories.is_empty());
    }

    #[test]
    fn remove_repo() {
        let mut ws = sample_workspace();
        let removed = ws.remove_repo("portal").unwrap();
        assert_eq!(removed.id, "portal");
        assert!(ws.get("portal").is_none());
        assert_eq!(ws.repositories.len(), 2);
        assert!(ws.remove_repo("missing").is_none());
    }

    #[test]
    fn set_enabled_toggles_and_validates() {
        let mut ws = sample_workspace();
        ws.set_enabled("payments", false).unwrap();
        assert!(!ws.get("payments").unwrap().enabled);
        assert_eq!(ws.enabled_repos().len(), 1);

        ws.set_enabled("payments", true).unwrap();
        assert!(ws.get("payments").unwrap().enabled);

        let err = ws.set_enabled("missing", true).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound(_)));

        let mut ws2 = WorkspaceConfig::default();
        ws2.repositories.push(
            RepositoryConfig::new("solo", "acme/solo", "https://example.com/solo.git")
                .with_branches(Vec::<String>::new())
                .with_enabled(false),
        );
        let err = ws2.set_enabled("solo", true).unwrap_err();
        assert!(matches!(err, ConfigError::NoBranches { .. }));
        assert!(!ws2.get("solo").unwrap().enabled);
    }

    #[test]
    fn set_branches_updates_and_validates() {
        let mut ws = sample_workspace();
        ws.set_branches("payments", ["main", "release", "develop"])
            .unwrap();
        assert_eq!(
            ws.get("payments").unwrap().monitored_branches,
            vec!["main", "release", "develop"]
        );

        let err = ws.set_branches("missing", ["main"]).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound(_)));

        ws.default_branches = Vec::new();
        let err = ws
            .set_branches("payments", Vec::<String>::new())
            .unwrap_err();
        assert!(matches!(err, ConfigError::NoBranches { .. }));
        assert_eq!(
            ws.get("payments").unwrap().monitored_branches,
            vec!["main", "release", "develop"]
        );
    }

    #[test]
    fn get_and_enabled_repos() {
        let ws = sample_workspace();
        assert!(ws.get("payments").is_some());
        assert!(ws.get("nope").is_none());
        let enabled: Vec<&str> = ws.enabled_repos().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(enabled, vec!["payments", "portal"]);
    }

    #[test]
    fn default_branches_roundtrip_and_default_empty() {
        let ws = WorkspaceConfig::default();
        assert!(ws.default_branches.is_empty());

        let raw = r#"
            default_branches = ["main", "develop"]
            [[repositories]]
            id = "api"
            github = "acme/api"
            clone_url = "https://github.com/acme/api.git"
        "#;
        let ws = WorkspaceConfig::from_toml(raw).unwrap();
        assert_eq!(ws.default_branches, vec!["main", "develop"]);

        let round = WorkspaceConfig::from_toml(&ws.to_toml().unwrap()).unwrap();
        assert_eq!(round.default_branches, ws.default_branches);
    }

    #[test]
    fn links_config_defaults_and_roundtrip() {
        let ws = WorkspaceConfig::default();
        assert!(ws.links.enabled);
        assert!(ws.links.base_urls.is_empty());
        assert!(ws.links.channel_brokers.is_empty());
        assert!(ws.links.resources.is_empty());

        let raw = r#"
            default_branches = ["main"]
            [links]
            enabled = false
            base_urls = { "https://api.orders.internal" = "orders-api" }
            channel_brokers = { "orders.created" = "kafka" }
            resources = { "s3://orders-dev/x" = "bucket:orders-prod" }

            [[repositories]]
            id = "api"
            github = "acme/api"
            clone_url = "https://github.com/acme/api.git"
        "#;
        let ws = WorkspaceConfig::from_toml(raw).unwrap();
        assert!(!ws.links.enabled);
        assert_eq!(
            ws.links.base_urls.get("https://api.orders.internal"),
            Some(&"orders-api".to_string())
        );
        assert_eq!(
            ws.links.channel_brokers.get("orders.created"),
            Some(&"kafka".to_string())
        );
        assert_eq!(
            ws.links.resources.get("s3://orders-dev/x"),
            Some(&"bucket:orders-prod".to_string())
        );

        let round = WorkspaceConfig::from_toml(&ws.to_toml().unwrap()).unwrap();
        assert_eq!(round.links, ws.links);
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let ws = WorkspaceConfig {
            repositories: vec![sample_repo("dup"), sample_repo("dup")],
            default_branches: Vec::new(),
            ..Default::default()
        };
        let err = ws.validate().unwrap_err();
        assert!(matches!(err, ConfigError::DuplicateId(id) if id == "dup"));
    }

    #[test]
    fn validate_rejects_branchless_enabled_repo() {
        let mut repo = sample_repo("ops");
        repo.monitored_branches = Vec::new();
        let err = repo.validate().unwrap_err();
        assert!(matches!(err, ConfigError::NoBranches { .. }));

        repo.enabled = false;
        assert!(repo.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_github_and_id() {
        let err = RepositoryConfig::new("x", "  ", "https://example.com/r.git")
            .validate()
            .unwrap_err();
        assert!(matches!(err, ConfigError::EmptyGithub { .. }));

        let err = RepositoryConfig::new(" ", "acme/r", "https://example.com/r.git")
            .validate()
            .unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn resolved_branches_precedence() {
        let mut ws = WorkspaceConfig {
            repositories: Vec::new(),
            default_branches: vec!["main".to_string(), "develop".to_string()],
            ..Default::default()
        };

        let explicit = sample_repo("payments").with_branches(["release"]);
        assert_eq!(
            ws.resolved_branches(&explicit),
            vec!["release".to_string()],
            "monitored_branches wins"
        );

        let empty = sample_repo("portal").with_branches(Vec::<String>::new());
        assert_eq!(
            ws.resolved_branches(&empty),
            vec!["main".to_string(), "develop".to_string()],
            "workspace default_branches used when monitored empty"
        );

        ws.default_branches = Vec::new();
        assert_eq!(
            ws.resolved_branches(&empty),
            vec!["main".to_string()],
            "fallback to main when both empty"
        );

        ws.default_branches = vec![String::new(), "  ".into()];
        assert_eq!(
            ws.resolved_branches(&empty),
            vec!["main".to_string()],
            "blank defaults ignored"
        );

        let blank_monitored = sample_repo("x").with_branches(["", " "]);
        assert_eq!(
            ws.resolved_branches(&blank_monitored),
            vec!["main".to_string()],
            "blank monitored entries ignored"
        );
    }

    #[test]
    fn default_branches_allow_empty_monitored_branches() {
        let raw = r#"
            default_branches = ["main", "develop"]

            [[repositories]]
            id = "api"
            github = "acme/api"
            clone_url = "https://github.com/acme/api.git"
            monitored_branches = []
        "#;
        let ws = WorkspaceConfig::from_toml(raw).unwrap();
        assert_eq!(
            ws.resolved_branches(ws.get("api").unwrap()),
            vec!["main".to_string(), "develop".to_string()]
        );
    }

    #[test]
    fn empty_monitored_without_defaults_still_rejected() {
        let raw = r#"
            [[repositories]]
            id = "api"
            github = "acme/api"
            clone_url = "https://github.com/acme/api.git"
            monitored_branches = []
        "#;
        let err = WorkspaceConfig::from_toml(raw).unwrap_err();
        assert!(matches!(err, ConfigError::NoBranches { .. }));
    }

    #[test]
    fn example_configs_parse_with_default_branches() {
        let example = WorkspaceConfig::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/workspace.example.toml"
        ))
        .unwrap();
        assert_eq!(
            example.default_branches,
            vec!["main".to_string(), "develop".to_string()]
        );

        let oss = WorkspaceConfig::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/workspace.oss.toml"
        ))
        .unwrap();
        assert!(oss.default_branches.is_empty());
        for repo in &oss.repositories {
            assert!(!repo.monitored_branches.is_empty());
        }
    }

    #[test]
    fn local_path_skips_empty_github_requirement() {
        let raw = r#"
            [[repositories]]
            id = "local-repo"
            github = ""
            clone_url = ""
            local_path = "/tmp/some-repo"
            monitored_branches = ["main"]
        "#;
        let ws = WorkspaceConfig::from_toml(raw).unwrap();
        let repo = ws.get("local-repo").unwrap();
        assert_eq!(repo.github_or_id(), "local-repo");
        assert_eq!(
            repo.canonical_id(),
            RepositoryConfig::new("local-repo", "local-repo", "").canonical_id()
        );
    }

    #[test]
    fn empty_local_path_rejected() {
        let raw = r#"
            [[repositories]]
            id = "local-repo"
            github = "acme/local-repo"
            clone_url = "https://example.com/r.git"
            local_path = "   "
            monitored_branches = ["main"]
        "#;
        let err = WorkspaceConfig::from_toml(raw).unwrap_err();
        assert!(matches!(err, ConfigError::Validation(_)));
    }

    #[test]
    fn empty_github_without_local_path_still_rejected() {
        let raw = r#"
            [[repositories]]
            id = "nope"
            github = ""
            clone_url = "https://example.com/r.git"
            monitored_branches = ["main"]
        "#;
        let err = WorkspaceConfig::from_toml(raw).unwrap_err();
        assert!(matches!(err, ConfigError::EmptyGithub { .. }));
    }
}
