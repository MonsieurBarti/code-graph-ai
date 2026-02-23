use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oxc_resolver::{AliasValue, ResolveOptions, Resolver, TsconfigOptions, TsconfigReferences};

/// The outcome of resolving a single import specifier.
#[derive(Debug)]
pub enum ResolutionOutcome {
    /// Successfully resolved to an absolute file path.
    Resolved(PathBuf),
    /// The specifier is a Node.js built-in module (e.g. `"fs"`, `"path"`, `"node:crypto"`).
    BuiltinModule(String),
    /// The specifier could not be resolved. `String` contains a human-readable reason.
    Unresolved(String),
}

/// Build an `oxc_resolver::Resolver` configured for TypeScript projects.
///
/// - TypeScript extensions are probed first (`.ts`, `.tsx`, `.mts`).
/// - `.js` extension aliases map to `.ts`/`.tsx`/`.js` so projects that write
///   `import './foo.js'` in TypeScript source resolve correctly.
/// - If `tsconfig.json` exists at `project_root`, path aliases and project references
///   are resolved automatically via `TsconfigReferences::Auto`.
/// - `workspace_aliases` are fed directly into `ResolveOptions::alias` so workspace
///   package names resolve to local source directories instead of `node_modules`.
pub fn build_resolver(
    project_root: &Path,
    workspace_aliases: Vec<(String, Vec<AliasValue>)>,
) -> Resolver {
    let tsconfig_path = project_root.join("tsconfig.json");
    let tsconfig = if tsconfig_path.exists() {
        Some(TsconfigOptions {
            config_file: tsconfig_path,
            references: TsconfigReferences::Auto,
        })
    } else {
        None
    };

    Resolver::new(ResolveOptions {
        extensions: vec![
            ".ts".into(),
            ".tsx".into(),
            ".mts".into(),
            ".js".into(),
            ".jsx".into(),
            ".mjs".into(),
            ".json".into(),
            ".node".into(),
        ],
        extension_alias: vec![(
            ".js".into(),
            vec![".ts".into(), ".tsx".into(), ".js".into()],
        )],
        tsconfig,
        alias: workspace_aliases,
        condition_names: vec!["node".into(), "import".into()],
        builtin_modules: true,
        ..ResolveOptions::default()
    })
}

/// Resolve a single import specifier from the perspective of `from_file`.
///
/// The resolver uses `from_file`'s parent directory as the resolution base, which matches
/// how Node.js and TypeScript resolve relative imports.
pub fn resolve_import(resolver: &Resolver, from_file: &Path, specifier: &str) -> ResolutionOutcome {
    let dir = match from_file.parent() {
        Some(d) => d,
        None => {
            return ResolutionOutcome::Unresolved("from_file has no parent directory".to_owned());
        }
    };

    match resolver.resolve(dir, specifier) {
        Ok(resolution) => ResolutionOutcome::Resolved(resolution.into_path_buf()),
        Err(oxc_resolver::ResolveError::Builtin { resolved, .. }) => {
            ResolutionOutcome::BuiltinModule(resolved)
        }
        Err(e) => ResolutionOutcome::Unresolved(e.to_string()),
    }
}

/// Convert a workspace package map into the alias format expected by `oxc_resolver`.
///
/// Each entry maps `package_name` → `[AliasValue::Path(source_dir)]`.
pub fn workspace_map_to_aliases(map: &HashMap<String, PathBuf>) -> Vec<(String, Vec<AliasValue>)> {
    map.iter()
        .map(|(name, path)| {
            (
                name.clone(),
                vec![AliasValue::Path(path.to_string_lossy().into_owned())],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Verify that build_resolver can be constructed without panicking even when
    /// no tsconfig.json exists and workspace aliases are empty.
    #[test]
    fn test_build_resolver_no_tsconfig_no_aliases() {
        // Use a temp dir that definitely has no tsconfig.json
        let dir = std::env::temp_dir();
        let resolver = build_resolver(&dir, vec![]);
        // If we get here without panicking, the resolver was created successfully.
        // Do a basic sanity check: resolve something that must succeed (a known path).
        let _outcome = resolve_import(&resolver, &dir.join("fake.ts"), "fs");
        // We don't assert on the outcome — we just verify no panic.
    }

    #[test]
    fn test_workspace_map_to_aliases_empty() {
        let map = HashMap::new();
        let aliases = workspace_map_to_aliases(&map);
        assert!(aliases.is_empty());
    }

    #[test]
    fn test_workspace_map_to_aliases_single_entry() {
        let mut map = HashMap::new();
        map.insert(
            "@myorg/utils".to_owned(),
            PathBuf::from("/repo/packages/utils/src"),
        );
        let aliases = workspace_map_to_aliases(&map);
        assert_eq!(aliases.len(), 1);
        let (name, values) = &aliases[0];
        assert_eq!(name, "@myorg/utils");
        assert_eq!(values.len(), 1);
        match &values[0] {
            AliasValue::Path(p) => assert_eq!(p, "/repo/packages/utils/src"),
            other => panic!("expected AliasValue::Path, got {:?}", other),
        }
    }
}
