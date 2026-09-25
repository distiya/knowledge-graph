use ckg_domain::{Analyzer, AnalyzerOutput, ProcessingStatus, Provenance};

/// SCIP analyzer adapter.
///
/// Full SCIP ingestion (definitions, references, implementations, and
/// cross-file semantic relationships) is future work and will be implemented
/// behind the same [`Analyzer`] trait. Until the analyzer is configured for
/// the MVP, calls return an empty output marked as skipped.
#[derive(Debug, Clone)]
pub struct ScipAnalyzer {
    version: String,
}

impl ScipAnalyzer {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

impl Default for ScipAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for ScipAnalyzer {
    fn name(&self) -> &'static str {
        "scip"
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn analyze_file(
        &self,
        _repository: &str,
        _commit_sha: &str,
        _path: &str,
        _content: &str,
        _language: &str,
    ) -> AnalyzerOutput {
        AnalyzerOutput {
            provenance: vec![
                Provenance::scip(self.version.as_str())
                    .with_status(ProcessingStatus::Skipped)
                    .with_detail("not configured for MVP"),
            ],
            ..AnalyzerOutput::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ckg_domain::ProvenanceSource;

    #[test]
    fn scip_analyzer_is_skipped_for_mvp() {
        let analyzer = ScipAnalyzer::new();
        assert_eq!(Analyzer::name(&analyzer), "scip");
        assert_eq!(analyzer.version(), env!("CARGO_PKG_VERSION"));

        let output = analyzer.analyze_file("org/repo", "abc", "src/lib.rs", "x", "rust");
        assert!(output.symbols.is_empty());
        assert!(output.relations.is_empty());
        assert!(output.errors.is_empty());
        assert_eq!(output.provenance.len(), 1);

        let provenance = &output.provenance[0];
        assert_eq!(provenance.source, ProvenanceSource::Scip);
        assert_eq!(provenance.status, ProcessingStatus::Skipped);
        assert_eq!(provenance.detail.as_deref(), Some("not configured for MVP"));
        assert_eq!(
            provenance.analyzer_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
