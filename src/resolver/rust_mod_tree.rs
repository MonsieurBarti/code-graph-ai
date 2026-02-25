use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tree_sitter::Tree;

use crate::parser::PARSER_RS;

/// A per-crate mapping of module paths to source file paths, and the reverse.
///
/// Built by walking `mod foo;` declarations from the crate root (lib.rs or main.rs).
/// Inline `mod foo { ... }` blocks are skipped — only file-backed declarations are followed.
///
/// Example:
/// - `"crate"` → `src/lib.rs`
/// - `"crate::parser"` → `src/parser/mod.rs`
/// - `"crate::parser::imports"` → `src/parser/imports.rs`
#[derive(Debug, Default)]
pub struct RustModTree {
    /// Module path → file path (e.g. `"crate::parser"` → `PathBuf("src/parser/mod.rs")`).
    pub mod_map: HashMap<String, PathBuf>,
    /// File path → module path (reverse map for `super::` resolution).
    pub reverse_map: HashMap<PathBuf, String>,
}

impl RustModTree {
    /// Look up a module path in the mod tree.
    ///
    /// If exact match fails, progressively strips the last `::` segment and retries.
    /// This handles `use crate::parser::imports::ImportKind` where `ImportKind` is a
    /// symbol name, not a module. Returns the deepest matching module file.
    pub fn resolve_module_path(&self, path: &str) -> Option<&PathBuf> {
        // Try exact match first.
        if let Some(file) = self.mod_map.get(path) {
            return Some(file);
        }

        // Progressively strip last segment.
        let mut current = path;
        loop {
            match current.rfind("::") {
                None => return None,
                Some(idx) => {
                    current = &current[..idx];
                    if let Some(file) = self.mod_map.get(current) {
                        return Some(file);
                    }
                }
            }
        }
    }

    /// Reverse lookup: given a file path, return its module path.
    ///
    /// Used for `super::` resolution — the reverse map tells us the current file's
    /// module position so we can compute the parent module path.
    pub fn file_to_module_path(&self, file: &Path) -> Option<&String> {
        // Try canonical path first (what walk_mod_tree stored).
        if let Some(mp) = self.reverse_map.get(file) {
            return Some(mp);
        }
        // Try with canonicalized path (resolves symlinks).
        if let Ok(canonical) = file.canonicalize() {
            return self.reverse_map.get(&canonical);
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Crate root detection
// ---------------------------------------------------------------------------

/// Find the crate root file (lib.rs or main.rs) and normalized crate name from a Cargo.toml path.
///
/// Returns `(crate_name, root_file_path)` where `crate_name` has hyphens replaced by
/// underscores (Cargo convention: `my-crate` → `my_crate` in `use` paths).
///
/// Lookup order:
/// 1. Explicit `[lib] path = "..."` in Cargo.toml
/// 2. `src/lib.rs` (preferred over main.rs for libraries)
/// 3. `src/main.rs` (binary crates)
pub fn find_crate_root(cargo_toml_path: &Path) -> Option<(String, PathBuf)> {
    let content = std::fs::read_to_string(cargo_toml_path).ok()?;
    let manifest: toml::Value = toml::from_str(&content).ok()?;

    // Get normalized crate name: hyphens → underscores.
    let raw_name = manifest.get("package")?.get("name")?.as_str()?;
    let crate_name = raw_name.replace('-', "_");

    let crate_dir = cargo_toml_path.parent()?;

    // 1. Check for explicit [lib] path = "..."
    if let Some(lib_path) = manifest
        .get("lib")
        .and_then(|l| l.get("path"))
        .and_then(|p| p.as_str())
    {
        let path = crate_dir.join(lib_path);
        if path.exists() {
            return Some((crate_name, path));
        }
    }

    // 2. Try src/lib.rs
    let lib_rs = crate_dir.join("src").join("lib.rs");
    if lib_rs.exists() {
        return Some((crate_name, lib_rs));
    }

    // 3. Try src/main.rs
    let main_rs = crate_dir.join("src").join("main.rs");
    if main_rs.exists() {
        return Some((crate_name, main_rs));
    }

    None
}

// ---------------------------------------------------------------------------
// Module declaration extraction
// ---------------------------------------------------------------------------

/// Extract all file-backed `mod foo;` declarations from a tree-sitter parse tree.
///
/// Only processes top-level children of the root node.
/// Skips inline `mod foo { ... }` blocks (those have a `body` child).
pub fn extract_mod_declarations(tree: &Tree, source: &[u8]) -> Vec<String> {
    let mut mods = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();

    for child in root.children(&mut cursor) {
        if child.kind() != "mod_item" {
            continue;
        }
        // Only file-backed mods: mod_item WITHOUT a body field.
        if child.child_by_field_name("body").is_some() {
            continue; // inline mod — skip
        }
        // Name field holds the module identifier.
        if let Some(name_node) = child.child_by_field_name("name") {
            let name = name_node.utf8_text(source).unwrap_or("").to_owned();
            if !name.is_empty() {
                mods.push(name);
            }
        }
    }

    mods
}

// ---------------------------------------------------------------------------
// Recursive module tree walker
// ---------------------------------------------------------------------------

/// Recursively walk the module tree from a single file, populating mod_map and reverse_map.
///
/// - `current_path`: the current module path (e.g. `"crate"` for the root, `"crate::parser"` for a submodule)
/// - `file`: the file path for the current module
/// - `mod_map`: mutable map being populated
/// - `reverse_map`: mutable reverse map being populated
/// - `visited`: cycle guard — canonicalized file paths already processed
///
/// For each `mod foo;` declaration found in `file`:
/// - Probes `{dir}/foo.rs` then `{dir}/foo/mod.rs` (Edition 2018+ layout)
/// - Recurses with path `{current_path}::foo` if a file is found
pub fn walk_mod_tree(
    current_path: &str,
    file: &Path,
    mod_map: &mut HashMap<String, PathBuf>,
    reverse_map: &mut HashMap<PathBuf, String>,
    visited: &mut HashSet<PathBuf>,
) {
    // Cycle guard: use canonicalized path if available, otherwise raw path.
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return; // already visited — stop recursion
    }

    // Record this file in both maps.
    mod_map.insert(current_path.to_string(), file.to_path_buf());
    reverse_map.insert(file.to_path_buf(), current_path.to_string());
    // Also insert canonical form if different.
    if canonical != file.to_path_buf() {
        reverse_map.insert(canonical, current_path.to_string());
    }

    // Read and parse the file.
    let source = match std::fs::read(file) {
        Ok(bytes) => bytes,
        Err(_) => return, // unreadable — continue gracefully
    };

    let tree = PARSER_RS.with(|p| p.borrow_mut().parse(&source, None));
    let tree = match tree {
        Some(t) => t,
        None => return, // parse failed — continue gracefully
    };

    let mod_names = extract_mod_declarations(&tree, &source);
    let parent_dir = match file.parent() {
        Some(d) => d,
        None => return,
    };

    // Determine the directory in which sub-module files live.
    //
    // Rust module system rules (Edition 2018+):
    //
    // 1. Crate root files (`lib.rs`, `main.rs`) and `mod.rs` files act as "directory owners":
    //    their sub-modules live in the same directory as themselves.
    //    e.g. `src/lib.rs` declares `mod parser;` → look for `src/parser.rs` or `src/parser/mod.rs`
    //
    // 2. Non-root, non-mod.rs files own a sub-directory named after their stem:
    //    e.g. `src/parser.rs` declares `mod imports;` → look for `src/parser/imports.rs`
    //
    // "Directory owner" files: mod.rs, lib.rs, main.rs
    let file_name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let is_directory_owner = matches!(file_name, "mod.rs" | "lib.rs" | "main.rs");
    let sub_dir = if is_directory_owner {
        parent_dir.to_path_buf()
    } else {
        // Non-root, non-mod.rs file: sub-modules live under a dir named after the file stem.
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        parent_dir.join(stem)
    };

    for mod_name in mod_names {
        // Edition 2018+ layout: probe foo.rs first, then foo/mod.rs.
        let candidate_file = sub_dir.join(format!("{mod_name}.rs"));
        let candidate_dir = sub_dir.join(&mod_name).join("mod.rs");

        let child_file = if candidate_file.exists() {
            candidate_file
        } else if candidate_dir.exists() {
            candidate_dir
        } else {
            // Missing file — log at verbose level and continue without panic.
            // (Callers that want verbose logging do so at a higher level.)
            continue;
        };

        let child_path = format!("{current_path}::{mod_name}");
        walk_mod_tree(&child_path, &child_file, mod_map, reverse_map, visited);
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Build the complete module tree for a crate, starting from its root file.
///
/// Returns a [`RustModTree`] with all `mod foo;` declarations resolved to file paths.
///
/// # Parameters
/// - `crate_name`: normalized crate name (hyphens replaced by underscores)
/// - `crate_root`: path to the crate's root source file (lib.rs or main.rs)
pub fn build_mod_tree(crate_name: &str, crate_root: &Path) -> RustModTree {
    let _ = crate_name; // crate name context is in the module paths ("crate::" prefix)
    let mut mod_map = HashMap::new();
    let mut reverse_map = HashMap::new();
    let mut visited = HashSet::new();

    walk_mod_tree(
        "crate",
        crate_root,
        &mut mod_map,
        &mut reverse_map,
        &mut visited,
    );

    RustModTree {
        mod_map,
        reverse_map,
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a simple crate layout in a tempdir:
    ///   src/lib.rs       — declares `mod parser;` and `mod utils;`
    ///   src/parser.rs    — declares `mod imports;`
    ///   src/parser/      — does NOT exist (parser is a file, not a dir)
    ///   src/utils.rs     — no sub-modules
    fn make_simple_crate(root: &Path) {
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub mod parser;\npub mod utils;\n").unwrap();
        fs::write(root.join("src/parser.rs"), "pub mod imports;\n").unwrap();
        fs::create_dir_all(root.join("src/parser")).unwrap();
        fs::write(root.join("src/parser/imports.rs"), "// imports module\n").unwrap();
        fs::write(root.join("src/utils.rs"), "// utils module\n").unwrap();
        // Write Cargo.toml
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"my-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
    }

    #[test]
    fn test_find_crate_root_lib() {
        let tmp = tempfile::tempdir().unwrap();
        make_simple_crate(tmp.path());
        let (name, root) = find_crate_root(&tmp.path().join("Cargo.toml")).unwrap();
        assert_eq!(name, "my_crate");
        assert!(root.ends_with("src/lib.rs"));
    }

    #[test]
    fn test_find_crate_root_main() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(p.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"my-bin\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let (name, root) = find_crate_root(&p.join("Cargo.toml")).unwrap();
        assert_eq!(name, "my_bin");
        assert!(root.ends_with("src/main.rs"));
    }

    #[test]
    fn test_build_mod_tree_maps_all_modules() {
        let tmp = tempfile::tempdir().unwrap();
        make_simple_crate(tmp.path());
        let crate_root = tmp.path().join("src/lib.rs");
        let tree = build_mod_tree("my_crate", &crate_root);

        assert!(
            tree.mod_map.contains_key("crate"),
            "crate root must be in map"
        );
        assert!(
            tree.mod_map.contains_key("crate::parser"),
            "crate::parser must be in map"
        );
        assert!(
            tree.mod_map.contains_key("crate::utils"),
            "crate::utils must be in map"
        );
        assert!(
            tree.mod_map.contains_key("crate::parser::imports"),
            "crate::parser::imports must be in map"
        );
    }

    #[test]
    fn test_resolve_module_path_strips_symbol_segment() {
        let tmp = tempfile::tempdir().unwrap();
        make_simple_crate(tmp.path());
        let crate_root = tmp.path().join("src/lib.rs");
        let tree = build_mod_tree("my_crate", &crate_root);

        // "crate::parser::imports::SomeSymbol" should resolve to imports.rs
        let result = tree.resolve_module_path("crate::parser::imports::SomeSymbol");
        assert!(
            result.is_some(),
            "should resolve by stripping symbol segment"
        );
        assert!(
            result.unwrap().ends_with("imports.rs"),
            "should resolve to imports.rs"
        );
    }

    #[test]
    fn test_reverse_map_populated() {
        let tmp = tempfile::tempdir().unwrap();
        make_simple_crate(tmp.path());
        let crate_root = tmp.path().join("src/lib.rs");
        let tree = build_mod_tree("my_crate", &crate_root);

        let utils_path = tmp.path().join("src/utils.rs");
        let mod_path = tree.file_to_module_path(&utils_path);
        assert_eq!(mod_path.map(|s| s.as_str()), Some("crate::utils"));
    }

    #[test]
    fn test_inline_mod_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path();
        std::fs::create_dir_all(p.join("src")).unwrap();
        // lib.rs has inline mod (should be skipped) and file-backed mod
        std::fs::write(
            p.join("src/lib.rs"),
            "mod inline { pub fn foo() {} }\npub mod file_backed;\n",
        )
        .unwrap();
        std::fs::write(p.join("src/file_backed.rs"), "// file-backed module\n").unwrap();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"test-crate\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let tree = build_mod_tree("test_crate", &p.join("src/lib.rs"));
        // inline should NOT create a file probe (no src/inline.rs file either)
        assert!(
            !tree.mod_map.contains_key("crate::inline"),
            "inline mod must not be in mod_map"
        );
        assert!(
            tree.mod_map.contains_key("crate::file_backed"),
            "file-backed mod must be in mod_map"
        );
    }
}
