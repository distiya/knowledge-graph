use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Stable canonical identity for a logical symbol/entity, independent of analyzer.
///
/// Identity incorporates: repository, namespace/package, container type,
/// symbol name, signature (optional), and language — never analyzer-specific IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CanonicalId(String);

impl Default for CanonicalId {
    fn default() -> Self {
        Self(String::new())
    }
}

impl CanonicalId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Build a deterministic canonical id from ordered components.
    ///
    /// Components are joined with `::` then hashed for a fixed-size stable id.
    pub fn from_parts(parts: &[&str]) -> Self {
        let joined = parts
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("::");
        Self::hash_str(&joined)
    }

    /// Deterministic SHA-256 based id (hex, truncated to 64 chars = full sha256).
    pub fn hash_str(input: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        Self(hex::encode(hasher.finalize()))
    }
}

impl std::fmt::Display for CanonicalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for CanonicalId {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl From<String> for CanonicalId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// Builder for constructing canonical symbol identities consistently.
#[derive(Debug, Default, Clone)]
pub struct CanonicalIdBuilder {
    repo: Option<String>,
    namespace: Option<String>,
    container: Option<String>,
    symbol: Option<String>,
    signature: Option<String>,
    language: Option<String>,
}

impl CanonicalIdBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn repository(mut self, repo: impl Into<String>) -> Self {
        self.repo = Some(repo.into());
        self
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn container(mut self, c: impl Into<String>) -> Self {
        self.container = Some(c.into());
        self
    }

    pub fn symbol(mut self, s: impl Into<String>) -> Self {
        self.symbol = Some(s.into());
        self
    }

    pub fn signature(mut self, s: impl Into<String>) -> Self {
        self.signature = Some(s.into());
        self
    }

    pub fn language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }

    pub fn build(self) -> CanonicalId {
        let mut parts: Vec<String> = Vec::new();
        for part in [
            self.language,
            self.repo,
            self.namespace,
            self.container,
            self.symbol,
            self.signature,
        ]
        .into_iter()
        .flatten()
        {
            if !part.is_empty() {
                parts.push(part);
            }
        }
        let joined = parts.join("::");
        CanonicalId::hash_str(&joined)
    }
}

/// Content hash of an entity's implementation body (not its identity).
pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_parts_same_id() {
        let a = CanonicalIdBuilder::new()
            .repository("org/svc")
            .namespace("pkg")
            .container("Foo")
            .symbol("bar")
            .language("rust")
            .build();
        let b = CanonicalIdBuilder::new()
            .repository("org/svc")
            .namespace("pkg")
            .container("Foo")
            .symbol("bar")
            .language("rust")
            .build();
        assert_eq!(a, b);
    }

    #[test]
    fn different_symbol_different_id() {
        let a = CanonicalIdBuilder::new()
            .repository("org/svc")
            .symbol("foo")
            .build();
        let b = CanonicalIdBuilder::new()
            .repository("org/svc")
            .symbol("bar")
            .build();
        assert_ne!(a, b);
    }

    #[test]
    fn content_hash_is_stable() {
        assert_eq!(content_hash(b"hello"), content_hash(b"hello"));
        assert_ne!(content_hash(b"hello"), content_hash(b"world"));
    }
}
