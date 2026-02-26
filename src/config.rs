use std::path::Path;

use serde::Deserialize;

/// MCP-specific configuration parsed from the `[mcp]` section of `code-graph.toml`.
#[derive(Debug, Deserialize, Clone)]
pub struct McpConfig {
    /// Default result limit for find_symbol, find_references, get_impact (default: 20).
    ///
    /// Per-call `limit` parameters override this value.
    #[serde(default = "default_limit")]
    pub default_limit: usize,

    /// Default sections filter for get_context (e.g. `"r,c"`). `None` = all sections.
    ///
    /// Per-call `sections` parameters override this value.
    pub default_sections: Option<String>,

    /// When `true`, suppress the `"truncated: N/total"` prefix lines from
    /// find_symbol, find_references, and get_impact output.
    #[serde(default)]
    pub suppress_summary_line: bool,
}

fn default_limit() -> usize {
    20
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            default_limit: default_limit(),
            default_sections: None,
            suppress_summary_line: false,
        }
    }
}

/// Configuration loaded from `code-graph.toml` at the project root.
#[derive(Debug, Deserialize, Default)]
pub struct CodeGraphConfig {
    /// Additional path patterns to exclude from indexing (beyond .gitignore and node_modules).
    pub exclude: Option<Vec<String>>,

    /// MCP tool behaviour defaults (limit, sections, truncation suppression).
    #[serde(default)]
    pub mcp: McpConfig,
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

    // CFG-01: empty TOML -> McpConfig has built-in defaults
    #[test]
    fn test_mcp_config_defaults() {
        let cfg = parse_config("");
        assert_eq!(cfg.mcp.default_limit, 20, "default_limit should be 20");
        assert!(
            cfg.mcp.default_sections.is_none(),
            "default_sections should be None"
        );
        assert!(
            !cfg.mcp.suppress_summary_line,
            "suppress_summary_line should be false"
        );
    }

    // CFG-01: all [mcp] fields present -> values respected
    #[test]
    fn test_mcp_config_full() {
        let toml_str = r#"
[mcp]
default_limit = 50
default_sections = "r,c"
suppress_summary_line = true
"#;
        let cfg = parse_config(toml_str);
        assert_eq!(cfg.mcp.default_limit, 50);
        assert_eq!(cfg.mcp.default_sections.as_deref(), Some("r,c"));
        assert!(cfg.mcp.suppress_summary_line);
    }

    // CFG-01: partial [mcp] section -> specified value respected, rest default
    #[test]
    fn test_mcp_config_partial() {
        let toml_str = r#"
[mcp]
default_limit = 50
"#;
        let cfg = parse_config(toml_str);
        assert_eq!(cfg.mcp.default_limit, 50, "explicit limit should be 50");
        assert!(
            cfg.mcp.default_sections.is_none(),
            "default_sections should be None (not set)"
        );
        assert!(
            !cfg.mcp.suppress_summary_line,
            "suppress_summary_line should default to false"
        );
    }

    // CFG-01: no [mcp] section -> McpConfig defaults used
    #[test]
    fn test_mcp_config_absent() {
        let toml_str = r#"exclude = ["foo"]"#;
        let cfg = parse_config(toml_str);
        assert_eq!(
            cfg.mcp.default_limit, 20,
            "should use built-in default when [mcp] absent"
        );
        assert!(cfg.mcp.default_sections.is_none());
        assert!(!cfg.mcp.suppress_summary_line);
    }

    // CFG-01: invalid TOML value in [mcp] -> falls back to CodeGraphConfig::default()
    // This test verifies the load() fallback path rather than direct deserialization.
    #[test]
    fn test_mcp_config_invalid_type() {
        // toml::from_str fails when default_limit = "not_a_number" because the field expects usize.
        let toml_str = r#"
[mcp]
default_limit = "not_a_number"
"#;
        let result = toml::from_str::<CodeGraphConfig>(toml_str);
        assert!(
            result.is_err(),
            "parsing invalid type for default_limit should fail, triggering load() fallback"
        );
    }
}
