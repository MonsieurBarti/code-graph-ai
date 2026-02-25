use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::resolver::rust_mod_tree::find_crate_root;

/// Discover all Rust crates in a project and return a map of normalized crate name → crate root file.
///
/// Handles three cases:
/// 1. **Workspace project**: Reads `[workspace].members` globs from the root `Cargo.toml`,
///    expands each pattern, finds each member's `Cargo.toml`, and calls `find_crate_root` on it.
/// 2. **Virtual workspace with package**: If the workspace `Cargo.toml` also has a `[package]`
///    section, includes that package as a member too.
/// 3. **Single-crate project**: Falls back to calling `find_crate_root` on the root `Cargo.toml`.
///
/// Crate names are normalized: hyphens → underscores (Cargo convention).
///
/// # Parameters
/// - `project_root`: the root directory of the project (where the top-level `Cargo.toml` lives)
///
/// # Returns
/// A map of `crate_name → crate_root_file_path`.
pub fn discover_rust_workspace_members(project_root: &Path) -> HashMap<String, PathBuf> {
    let workspace_toml = project_root.join("Cargo.toml");

    let content = match std::fs::read_to_string(&workspace_toml) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let manifest: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let mut result: HashMap<String, PathBuf> = HashMap::new();

    // Check if this is a workspace Cargo.toml.
    let workspace_members = manifest
        .get("workspace")
        .and_then(|ws| ws.get("members"))
        .and_then(|m| m.as_array());

    if let Some(members) = workspace_members {
        // Expand workspace member glob patterns.
        for member_value in members {
            let member_glob = match member_value.as_str() {
                Some(s) => s,
                None => continue,
            };

            // Build the full glob pattern: <project_root>/<member_pattern>/Cargo.toml
            let pattern = format!("{}/{}/Cargo.toml", project_root.display(), member_glob);

            let entries = match glob::glob(&pattern) {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                if let Some((name, root)) = find_crate_root(&entry) {
                    result.insert(name, root);
                }
            }
        }

        // Handle combined workspace+package Cargo.toml (virtual workspace with root package).
        if manifest.get("package").is_some()
            && let Some((name, root)) = find_crate_root(&workspace_toml)
        {
            result.entry(name).or_insert(root);
        }

        result
    } else {
        // Single-crate project: just find the root crate.
        if let Some((name, root)) = find_crate_root(&workspace_toml) {
            result.insert(name, root);
        }
        result
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_workspace(root: &Path) {
        // Root workspace Cargo.toml
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();

        // crates/alpha/
        let alpha = root.join("crates/alpha");
        fs::create_dir_all(alpha.join("src")).unwrap();
        fs::write(
            alpha.join("Cargo.toml"),
            "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(alpha.join("src/lib.rs"), "// alpha lib\n").unwrap();

        // crates/beta-utils/
        let beta = root.join("crates/beta-utils");
        fs::create_dir_all(beta.join("src")).unwrap();
        fs::write(
            beta.join("Cargo.toml"),
            "[package]\nname = \"beta-utils\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(beta.join("src/lib.rs"), "// beta lib\n").unwrap();
    }

    #[test]
    fn test_workspace_discovers_members() {
        let tmp = tempfile::tempdir().unwrap();
        make_workspace(tmp.path());
        let members = discover_rust_workspace_members(tmp.path());

        assert!(members.contains_key("alpha"), "alpha should be discovered");
        assert!(
            members.contains_key("beta_utils"),
            "beta-utils should be normalized to beta_utils"
        );
        assert_eq!(members.len(), 2, "should have exactly 2 workspace members");
    }

    #[test]
    fn test_single_crate_project() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"my-app\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(p.join("src/main.rs"), "fn main() {}").unwrap();

        let members = discover_rust_workspace_members(p);
        assert!(
            members.contains_key("my_app"),
            "single crate should be discovered as my_app"
        );
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn test_missing_cargo_toml_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let members = discover_rust_workspace_members(tmp.path());
        assert!(
            members.is_empty(),
            "missing Cargo.toml should return empty map"
        );
    }
}
