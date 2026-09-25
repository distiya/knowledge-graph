use ckg_domain::{Analyzer, AnalyzerOutput, ProcessingStatus, Provenance};

/// Joern analyzer adapter.
///
/// Full Joern integration (AST, CFG, DFG, call-graph, and data-flow analysis)
/// is future work and will be implemented behind the same [`Analyzer`] trait.
/// Until the analyzer is configured for the MVP, calls return an empty output
/// marked as skipped.
#[derive(Debug, Clone)]
pub struct JoernAnalyzer {
    version: String,
}

impl JoernAnalyzer {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn version(&self) -> &str {
        &self.version
    }
}

impl Default for JoernAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer for JoernAnalyzer {
    fn name(&self) -> &'static str {
        "joern"
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
                Provenance::joern(self.version.as_str())
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
    fn joern_analyzer_is_skipped_for_mvp() {
        let analyzer = JoernAnalyzer::new();
        assert_eq!(Analyzer::name(&analyzer), "joern");
        assert_eq!(analyzer.version(), env!("CARGO_PKG_VERSION"));

        let output = analyzer.analyze_file("org/repo", "abc", "src/lib.rs", "x", "rust");
        assert!(output.symbols.is_empty());
        assert!(output.relations.is_empty());
        assert!(output.errors.is_empty());
        assert_eq!(output.provenance.len(), 1);

        let provenance = &output.provenance[0];
        assert_eq!(provenance.source, ProvenanceSource::Joern);
        assert_eq!(provenance.status, ProcessingStatus::Skipped);
        assert_eq!(provenance.detail.as_deref(), Some("not configured for MVP"));
        assert_eq!(
            provenance.analyzer_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
