use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Discover workspace packages for npm/yarn/pnpm monorepos.
///
/// Returns a map from package name (e.g. `"@myorg/utils"`) to the package's source directory
/// (prefers `<pkg>/src/` when it exists, otherwise `<pkg>/` root). Returns an empty map when
/// no workspace configuration is found.
pub fn discover_workspace_packages(root: &Path) -> HashMap<String, PathBuf> {
    let mut result = HashMap::new();
    let patterns = read_workspace_globs(root);

    for pattern in patterns {
        let full_pattern = format!("{}/{}/package.json", root.display(), pattern);
        if let Ok(paths) = glob::glob(&full_pattern) {
            for pkg_json_path in paths.flatten() {
                if let Some(pkg_dir) = pkg_json_path.parent() {
                    if let Ok(content) = std::fs::read_to_string(&pkg_json_path) {
                        if let Ok(json) =
                            serde_json::from_str::<serde_json::Value>(&content)
                        {
                            if let Some(name) = json["name"].as_str() {
                                let src = pkg_dir.join("src");
                                let target =
                                    if src.exists() { src } else { pkg_dir.to_path_buf() };
                                result.insert(name.to_owned(), target);
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

/// Read workspace glob patterns from the project root.
///
/// Checks for pnpm-workspace.yaml first; falls back to package.json workspaces field.
fn read_workspace_globs(root: &Path) -> Vec<String> {
    // pnpm: pnpm-workspace.yaml with 'packages:' array
    let pnpm_yaml = root.join("pnpm-workspace.yaml");
    if pnpm_yaml.exists() {
        if let Ok(content) = std::fs::read_to_string(&pnpm_yaml) {
            return parse_pnpm_workspace_yaml(&content);
        }
    }

    // npm/yarn: package.json with 'workspaces' array
    let pkg_json = root.join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_json) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(arr) = json["workspaces"].as_array() {
                return arr.iter().filter_map(|v| v.as_str().map(String::from)).collect();
            }
        }
    }

    vec![]
}

/// Minimal YAML line parser for pnpm-workspace.yaml.
///
/// The pnpm workspace YAML format is simple — it contains a `packages:` key followed by
/// a list of glob strings. This parser handles the common cases without requiring a full
/// YAML dependency:
///
/// ```yaml
/// packages:
///   - 'packages/*'
///   - "apps/*"
///   - tools/*
/// ```
pub(crate) fn parse_pnpm_workspace_yaml(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut in_packages = false;

    for line in content.lines() {
        let trimmed = line.trim_end();

        if trimmed == "packages:" || trimmed == "packages: " {
            in_packages = true;
            continue;
        }

        if in_packages {
            // A new top-level key (no leading whitespace, ends with ':') ends the packages block
            if !trimmed.is_empty() && !trimmed.starts_with(' ') && !trimmed.starts_with('-') {
                break;
            }

            // Match list items: '  - value' or '  - "value"' or "  - 'value'"
            let stripped = trimmed.trim_start();
            if let Some(rest) = stripped.strip_prefix("- ") {
                let glob = rest.trim();
                // Strip surrounding quotes (single or double)
                let glob = if (glob.starts_with('\'') && glob.ends_with('\''))
                    || (glob.starts_with('"') && glob.ends_with('"'))
                {
                    &glob[1..glob.len() - 1]
                } else {
                    glob
                };
                if !glob.is_empty() {
                    result.push(glob.to_owned());
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pnpm_workspace_yaml_single_quotes() {
        let yaml = "packages:\n  - 'packages/*'\n  - 'apps/*'\n";
        let globs = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(globs, vec!["packages/*", "apps/*"]);
    }

    #[test]
    fn test_parse_pnpm_workspace_yaml_double_quotes() {
        let yaml = "packages:\n  - \"packages/*\"\n  - \"apps/*\"\n";
        let globs = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(globs, vec!["packages/*", "apps/*"]);
    }

    #[test]
    fn test_parse_pnpm_workspace_yaml_no_quotes() {
        let yaml = "packages:\n  - packages/*\n  - tools/*\n";
        let globs = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(globs, vec!["packages/*", "tools/*"]);
    }

    #[test]
    fn test_parse_pnpm_workspace_yaml_with_other_keys() {
        let yaml = "packages:\n  - 'packages/*'\nnpmrc:\n  registry: https://registry.npmjs.org\n";
        let globs = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(globs, vec!["packages/*"]);
    }

    #[test]
    fn test_parse_pnpm_workspace_yaml_empty() {
        let yaml = "packages:\n";
        let globs = parse_pnpm_workspace_yaml(yaml);
        assert!(globs.is_empty());
    }

    #[test]
    fn test_parse_pnpm_workspace_yaml_mixed_quotes() {
        let yaml = "packages:\n  - 'packages/*'\n  - \"apps/*\"\n  - shared/*\n";
        let globs = parse_pnpm_workspace_yaml(yaml);
        assert_eq!(globs, vec!["packages/*", "apps/*", "shared/*"]);
    }
}
