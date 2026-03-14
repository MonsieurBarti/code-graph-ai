use std::path::Path;

use serde::Deserialize;

/// Impact analysis configuration parsed from the `[impact]` section of `code-graph.toml`.
#[derive(Debug, Deserialize, Clone)]
pub struct ImpactConfig {
    /// Files above this count are classified as HIGH risk (default: 20).
    #[serde(default = "default_high_threshold")]
    pub high_threshold: usize,
    /// Files at or above this count (but below high) are MEDIUM risk (default: 5).
    #[serde(default = "default_medium_threshold")]
    pub medium_threshold: usize,
}

fn default_high_threshold() -> usize {
    20
}
fn default_medium_threshold() -> usize {
    5
}

impl Default for ImpactConfig {
    fn default() -> Self {
        Self {
            high_threshold: default_high_threshold(),
            medium_threshold: default_medium_threshold(),
        }
    }
}

/// Configuration loaded from `code-graph.toml` at the project root.
#[derive(Debug, Deserialize, Default)]
pub struct CodeGraphConfig {
    /// Additional path patterns to exclude from indexing (beyond .gitignore and node_modules).
    pub exclude: Option<Vec<String>>,

    /// Impact analysis configuration (thresholds for risk tiers).
    #[serde(default)]
    pub impact: ImpactConfig,
}

impl CodeGraphConfig {
    /// Load configuration from `code-graph.toml` in the given root directory.
    ///
    /// Returns a default (empty) configuration if the file does not exist or cannot be parsed.
    pub fn load(root: &Path) -> Self {
        let config_path = root.join("code-graph.toml");

        if !config_path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(&config_path) {
            Ok(contents) => match toml::from_str::<Self>(&contents) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("warning: failed to parse code-graph.toml: {err}. Using defaults.");
                    Self::default()
                }
            },
            Err(err) => {
                eprintln!("warning: failed to read code-graph.toml: {err}. Using defaults.");
                Self::default()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_config(toml_str: &str) -> CodeGraphConfig {
        toml::from_str(toml_str).expect("TOML should parse without error")
    }

    // IMPACT-01: Default ImpactConfig has high_threshold=20, medium_threshold=5
    #[test]
    fn test_impact_config_defaults() {
        let cfg = parse_config("");
        assert_eq!(
            cfg.impact.high_threshold, 20,
            "default high_threshold should be 20"
        );
        assert_eq!(
            cfg.impact.medium_threshold, 5,
            "default medium_threshold should be 5"
        );
    }

    // IMPACT-01: Parsing [impact] section from TOML populates thresholds
    #[test]
    fn test_impact_config_from_toml() {
        let toml_str = r#"
[impact]
high_threshold = 30
medium_threshold = 10
"#;
        let cfg = parse_config(toml_str);
        assert_eq!(cfg.impact.high_threshold, 30, "high_threshold should be 30");
        assert_eq!(
            cfg.impact.medium_threshold, 10,
            "medium_threshold should be 10"
        );
    }

    // IMPACT-01: Partial [impact] section -> specified value respected, rest default
    #[test]
    fn test_impact_config_partial() {
        let toml_str = r#"
[impact]
high_threshold = 50
"#;
        let cfg = parse_config(toml_str);
        assert_eq!(
            cfg.impact.high_threshold, 50,
            "explicit high_threshold should be 50"
        );
        assert_eq!(
            cfg.impact.medium_threshold, 5,
            "medium_threshold should default to 5"
        );
    }
}
