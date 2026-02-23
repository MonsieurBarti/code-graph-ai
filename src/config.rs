use std::path::Path;

use serde::Deserialize;

/// Configuration loaded from `code-graph.toml` at the project root.
#[derive(Debug, Deserialize, Default)]
pub struct CodeGraphConfig {
    /// Additional path patterns to exclude from indexing (beyond .gitignore and node_modules).
    pub exclude: Option<Vec<String>>,
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
