use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Represents a programming language handled by code-graph.
///
/// Uses a plain enum (not trait objects) to avoid `dyn` overhead. Cheap to copy
/// and pattern-matched at dispatch boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LanguageKind {
    TypeScript,
    JavaScript,
    Rust,
    Python,
    Go,
}

impl LanguageKind {
    /// Returns true if this language kind matches a given file extension.
    pub fn matches_extension(&self, ext: &str) -> bool {
        match self {
            LanguageKind::TypeScript => matches!(ext, "ts" | "tsx"),
            LanguageKind::JavaScript => matches!(ext, "js" | "jsx"),
            LanguageKind::Rust => ext == "rs",
            LanguageKind::Python => ext == "py",
            LanguageKind::Go => ext == "go",
        }
    }

    /// Parse a CLI flag string into a `LanguageKind`. Case-insensitive.
    ///
    /// Accepted values:
    /// - "typescript" or "ts" -> TypeScript
    /// - "javascript" or "js" -> JavaScript
    /// - "rust" or "rs"       -> Rust
    pub fn from_str_loose(s: &str) -> Option<LanguageKind> {
        match s.to_lowercase().as_str() {
            "typescript" | "ts" => Some(LanguageKind::TypeScript),
            "javascript" | "js" => Some(LanguageKind::JavaScript),
            "rust" | "rs" => Some(LanguageKind::Rust),
            "python" | "py" => Some(LanguageKind::Python),
            "go" | "golang" => Some(LanguageKind::Go),
            _ => None,
        }
    }
}

/// Config files that signal a language's presence at a project root.
const CONFIG_FILES: &[(&str, LanguageKind)] = &[
    ("Cargo.toml", LanguageKind::Rust),
    ("tsconfig.json", LanguageKind::TypeScript),
    ("package.json", LanguageKind::JavaScript),
    ("pyproject.toml", LanguageKind::Python),
    ("setup.py", LanguageKind::Python),
    ("go.mod", LanguageKind::Go),
];

/// Detect which languages are present in a project root.
///
/// Strategy: check root + one level deep for config files. One-level-deep
/// detection catches monorepo workspace members.
///
/// Note: Extension-based detection is NOT done here (expensive full walk).
/// Instead, the walk result from `walk_project` is used post-walk to confirm
/// language presence by file extension. See `main.rs` integration.
///
/// Special case: when tsconfig.json AND package.json are both present,
/// TypeScript is detected (tsconfig.json signals a TS project even when
/// package.json exists). JavaScript is only emitted when package.json exists
/// WITHOUT a tsconfig.json.
pub fn detect_languages(root: &Path) -> HashSet<LanguageKind> {
    let mut found = HashSet::new();

    check_config_files(root, &mut found);

    // Check one level deep (monorepo workspace members).
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                check_config_files(&entry.path(), &mut found);
            }
        }
    }

    // If TypeScript is detected, remove the JavaScript entry that may have been
    // added due to package.json presence — tsconfig.json signals a TS project.
    // But we still keep both if some sub-directory is pure JS and another is TS.
    // The root-level rule: tsconfig.json supersedes package.json for root detection.
    if root.join("tsconfig.json").exists() {
        found.remove(&LanguageKind::JavaScript);
    }

    found
}

/// Check a single directory for config files and insert detected languages.
fn check_config_files(dir: &Path, found: &mut HashSet<LanguageKind>) {
    for (filename, lang) in CONFIG_FILES {
        if dir.join(filename).exists() {
            found.insert(*lang);
        }
    }
    // tsconfig.json in same dir supersedes package.json -> remove JavaScript
    if dir.join("tsconfig.json").exists() {
        found.remove(&LanguageKind::JavaScript);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn test_detect_rust_project() {
        let dir = tmp();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"foo\"").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&LanguageKind::Rust));
        assert!(!langs.contains(&LanguageKind::TypeScript));
        assert!(!langs.contains(&LanguageKind::JavaScript));
    }

    #[test]
    fn test_detect_ts_project() {
        let dir = tmp();
        fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&LanguageKind::TypeScript));
        assert!(!langs.contains(&LanguageKind::Rust));
        assert!(!langs.contains(&LanguageKind::JavaScript));
    }

    #[test]
    fn test_detect_js_project() {
        let dir = tmp();
        fs::write(dir.path().join("package.json"), "{}").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&LanguageKind::JavaScript));
        assert!(!langs.contains(&LanguageKind::TypeScript));
        assert!(!langs.contains(&LanguageKind::Rust));
    }

    #[test]
    fn test_detect_mixed_project() {
        let dir = tmp();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"foo\"").unwrap();
        fs::write(dir.path().join("tsconfig.json"), "{}").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&LanguageKind::Rust));
        assert!(langs.contains(&LanguageKind::TypeScript));
    }

    #[test]
    fn test_detect_one_level_deep() {
        let dir = tmp();
        // Direct subdirectory of root (one level deep).
        let sub = dir.path().join("mylib");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("Cargo.toml"), "[package]\nname = \"mylib\"").unwrap();
        let langs = detect_languages(dir.path());
        assert!(langs.contains(&LanguageKind::Rust));
    }

    #[test]
    fn test_matches_extension() {
        assert!(LanguageKind::TypeScript.matches_extension("ts"));
        assert!(LanguageKind::TypeScript.matches_extension("tsx"));
        assert!(!LanguageKind::TypeScript.matches_extension("js"));
        assert!(!LanguageKind::TypeScript.matches_extension("rs"));

        assert!(LanguageKind::JavaScript.matches_extension("js"));
        assert!(LanguageKind::JavaScript.matches_extension("jsx"));
        assert!(!LanguageKind::JavaScript.matches_extension("ts"));
        assert!(!LanguageKind::JavaScript.matches_extension("rs"));

        assert!(LanguageKind::Rust.matches_extension("rs"));
        assert!(!LanguageKind::Rust.matches_extension("ts"));
        assert!(!LanguageKind::Rust.matches_extension("js"));

        assert!(LanguageKind::Python.matches_extension("py"));
        assert!(!LanguageKind::Python.matches_extension("rs"));
        assert!(!LanguageKind::Python.matches_extension("ts"));

        assert!(LanguageKind::Go.matches_extension("go"));
        assert!(!LanguageKind::Go.matches_extension("rs"));
        assert!(!LanguageKind::Go.matches_extension("ts"));
    }

    #[test]
    fn test_from_str_loose() {
        assert_eq!(
            LanguageKind::from_str_loose("typescript"),
            Some(LanguageKind::TypeScript)
        );
        assert_eq!(
            LanguageKind::from_str_loose("ts"),
            Some(LanguageKind::TypeScript)
        );
        assert_eq!(
            LanguageKind::from_str_loose("TypeScript"),
            Some(LanguageKind::TypeScript)
        );
        assert_eq!(
            LanguageKind::from_str_loose("TS"),
            Some(LanguageKind::TypeScript)
        );

        assert_eq!(
            LanguageKind::from_str_loose("javascript"),
            Some(LanguageKind::JavaScript)
        );
        assert_eq!(
            LanguageKind::from_str_loose("js"),
            Some(LanguageKind::JavaScript)
        );
        assert_eq!(
            LanguageKind::from_str_loose("JavaScript"),
            Some(LanguageKind::JavaScript)
        );

        assert_eq!(
            LanguageKind::from_str_loose("rust"),
            Some(LanguageKind::Rust)
        );
        assert_eq!(LanguageKind::from_str_loose("rs"), Some(LanguageKind::Rust));
        assert_eq!(
            LanguageKind::from_str_loose("Rust"),
            Some(LanguageKind::Rust)
        );
        assert_eq!(LanguageKind::from_str_loose("RS"), Some(LanguageKind::Rust));

        assert_eq!(
            LanguageKind::from_str_loose("python"),
            Some(LanguageKind::Python)
        );
        assert_eq!(
            LanguageKind::from_str_loose("py"),
            Some(LanguageKind::Python)
        );
        assert_eq!(
            LanguageKind::from_str_loose("Python"),
            Some(LanguageKind::Python)
        );
        assert_eq!(LanguageKind::from_str_loose("go"), Some(LanguageKind::Go));
        assert_eq!(
            LanguageKind::from_str_loose("golang"),
            Some(LanguageKind::Go)
        );
        assert_eq!(LanguageKind::from_str_loose("Go"), Some(LanguageKind::Go));
        assert_eq!(LanguageKind::from_str_loose(""), None);
    }
}
