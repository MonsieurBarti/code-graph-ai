/// Persistent project registry stored at `~/.code-graph/projects.toml`.
///
/// Each entry maps a user-chosen alias to a canonicalized project path with metadata.
/// The registry supports CRUD operations: add, remove, list, get.
///
/// File format (hand-editable TOML):
/// ```toml
/// [projects.my-app]
/// path = "/Users/alice/projects/my-app"
/// added_at = 1710000000
///
/// [projects.backend]
/// path = "/Users/alice/work/backend"
/// added_at = 1710000100
/// ```
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

// ─── Types ────────────────────────────────────────────────────────────────────

/// A single registered project entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectEntry {
    /// User-chosen alias for this project.
    #[serde(skip)]
    pub alias: String,
    /// Canonicalized absolute path to the project root.
    pub path: PathBuf,
    /// Unix timestamp (seconds since epoch) when the project was registered.
    pub added_at: u64,
}

/// Top-level TOML structure for the registry file.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    /// Map from alias -> project entry.
    #[serde(default)]
    projects: BTreeMap<String, ProjectEntry>,
}

/// Manages the persistent project registry.
pub struct ProjectRegistry {
    /// Path to the `projects.toml` file.
    toml_path: PathBuf,
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

/// Returns the path to `~/.code-graph/projects.toml`.
///
/// Uses `HOME` (Unix) / `USERPROFILE` (Windows) env vars to locate the home directory.
fn registry_toml_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".code-graph")
        .join("projects.toml")
}

// ─── Alias validation ─────────────────────────────────────────────────────────

/// Validate that an alias conforms to the allowed pattern: `[a-zA-Z0-9-]{1,64}`.
fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty() {
        bail!("alias must not be empty");
    }
    if alias.len() > 64 {
        bail!(
            "alias '{}...' exceeds maximum length of 64 characters (got {})",
            &alias[..32],
            alias.len()
        );
    }
    if let Some(c) = alias
        .chars()
        .find(|c| !c.is_ascii_alphanumeric() && *c != '-')
    {
        bail!(
            "alias '{}' contains invalid character '{}' — only alphanumeric characters and hyphens are allowed",
            alias,
            c
        );
    }
    Ok(())
}

// ─── Registry implementation ──────────────────────────────────────────────────

impl ProjectRegistry {
    /// Create a new registry instance using the default path (`~/.code-graph/projects.toml`).
    pub fn new() -> Self {
        Self {
            toml_path: registry_toml_path(),
        }
    }

    /// Create a registry instance with a custom TOML path (useful for testing).
    #[cfg(test)]
    fn with_path(toml_path: PathBuf) -> Self {
        Self { toml_path }
    }

    /// Register a project with the given alias and path.
    ///
    /// The path is canonicalized before storage. Returns an error if:
    /// - The alias is invalid (see [`validate_alias`])
    /// - The path does not exist on disk
    /// - The alias is already registered
    /// - The same canonical path is already registered under a different alias
    pub fn add(&self, alias: &str, path: &Path) -> Result<ProjectEntry> {
        validate_alias(alias)?;

        let canonical = path.canonicalize().with_context(|| {
            format!(
                "path '{}' does not exist or is not accessible",
                path.display()
            )
        })?;

        let mut registry = self.load()?;

        // Check for duplicate alias.
        if registry.projects.contains_key(alias) {
            bail!(
                "alias '{}' is already registered — use a different alias or remove the existing one first",
                alias
            );
        }

        // Check for duplicate path.
        for (existing_alias, entry) in &registry.projects {
            if entry.path == canonical {
                bail!(
                    "path '{}' is already registered under alias '{}' — each project path can only be registered once",
                    canonical.display(),
                    existing_alias
                );
            }
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system time is before unix epoch")
            .as_secs();

        let entry = ProjectEntry {
            alias: alias.to_string(),
            path: canonical,
            added_at: now,
        };

        registry.projects.insert(alias.to_string(), entry.clone());

        self.save(&registry)?;

        Ok(entry)
    }

    /// Remove a project by alias. Returns an error if the alias is not found.
    pub fn remove(&self, alias: &str) -> Result<()> {
        let mut registry = self.load()?;

        if registry.projects.remove(alias).is_none() {
            bail!("alias '{}' is not registered — nothing to remove", alias);
        }

        self.save(&registry)?;
        Ok(())
    }

    /// List all registered projects, sorted by alias.
    pub fn list(&self) -> Vec<ProjectEntry> {
        let registry = self.load().unwrap_or_default();
        registry
            .projects
            .into_iter()
            .map(|(alias, mut entry)| {
                entry.alias = alias;
                entry
            })
            .collect()
    }

    /// Look up a project by alias. Returns `None` if not found.
    pub fn get(&self, alias: &str) -> Option<ProjectEntry> {
        let registry = self.load().unwrap_or_default();
        registry.projects.get(alias).map(|entry| {
            let mut e = entry.clone();
            e.alias = alias.to_string();
            e
        })
    }

    // ─── Internal helpers ─────────────────────────────────────────────────────

    /// Load the registry from disk. Returns a default (empty) registry if the file
    /// does not exist.
    fn load(&self) -> Result<RegistryFile> {
        if !self.toml_path.exists() {
            return Ok(RegistryFile::default());
        }

        let content = std::fs::read_to_string(&self.toml_path)
            .with_context(|| format!("failed to read {}", self.toml_path.display()))?;

        let registry: RegistryFile = toml::from_str(&content)
            .with_context(|| format!("failed to parse {}", self.toml_path.display()))?;

        Ok(registry)
    }

    /// Persist the registry to disk using atomic write (tmp file + rename).
    ///
    /// Creates the `~/.code-graph/` directory if it does not exist.
    fn save(&self, registry: &RegistryFile) -> Result<()> {
        // Ensure parent directory exists.
        if let Some(parent) = self.toml_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory {}", parent.display()))?;
        }

        let content =
            toml::to_string_pretty(registry).context("failed to serialize registry to TOML")?;

        // Atomic write: write to a tmp file in the same directory, then rename.
        let tmp_path = self.toml_path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, content.as_bytes())
            .with_context(|| format!("failed to write temporary file {}", tmp_path.display()))?;

        std::fs::rename(&tmp_path, &self.toml_path).with_context(|| {
            format!(
                "failed to rename {} to {}",
                tmp_path.display(),
                self.toml_path.display()
            )
        })?;

        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Create a registry that stores its TOML in a temp directory.
    fn test_registry(tmp: &TempDir) -> ProjectRegistry {
        let toml_path = tmp.path().join(".code-graph").join("projects.toml");
        ProjectRegistry::with_path(toml_path)
    }

    /// Create a real directory inside tmp that can be canonicalized.
    fn create_project_dir(tmp: &TempDir, name: &str) -> PathBuf {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).expect("create project dir");
        dir.canonicalize().expect("canonicalize project dir")
    }

    // ── add + list round-trip ─────────────────────────────────────────────

    #[test]
    fn add_and_list_round_trip() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let project_dir = create_project_dir(&tmp, "my-project");

        let entry = reg.add("my-project", &project_dir).unwrap();
        assert_eq!(entry.alias, "my-project");
        assert_eq!(entry.path, project_dir);
        assert!(entry.added_at > 0);

        let entries = reg.list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "my-project");
        assert_eq!(entries[0].path, project_dir);
    }

    // ── duplicate alias rejection ─────────────────────────────────────────

    #[test]
    fn add_duplicate_alias_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir1 = create_project_dir(&tmp, "proj-a");
        let dir2 = create_project_dir(&tmp, "proj-b");

        reg.add("my-alias", &dir1).unwrap();
        let err = reg.add("my-alias", &dir2).unwrap_err();
        assert!(
            err.to_string().contains("already registered"),
            "expected 'already registered' in error: {}",
            err
        );
    }

    // ── duplicate path rejection ──────────────────────────────────────────

    #[test]
    fn add_duplicate_path_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "proj");

        reg.add("alias-one", &dir).unwrap();
        let err = reg.add("alias-two", &dir).unwrap_err();
        assert!(
            err.to_string().contains("already registered under alias"),
            "expected path-duplicate error, got: {}",
            err
        );
    }

    // ── alias validation: valid cases ─────────────────────────────────────

    #[test]
    fn alias_validation_accepts_valid_aliases() {
        // These should all pass.
        assert!(validate_alias("a").is_ok());
        assert!(validate_alias("my-project").is_ok());
        assert!(validate_alias("MyProject123").is_ok());
        assert!(validate_alias("A").is_ok());
        assert!(validate_alias("a-b-c-d").is_ok());
        // 64 chars exactly should be fine.
        let max_alias = "a".repeat(64);
        assert!(validate_alias(&max_alias).is_ok());
    }

    // ── alias validation: invalid cases ───────────────────────────────────

    #[test]
    fn alias_validation_rejects_empty() {
        let err = validate_alias("").unwrap_err();
        assert!(
            err.to_string().contains("must not be empty"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn alias_validation_rejects_too_long() {
        let long_alias = "a".repeat(65);
        let err = validate_alias(&long_alias).unwrap_err();
        assert!(
            err.to_string().contains("exceeds maximum length"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn alias_validation_rejects_invalid_chars() {
        for bad in &[
            "my project",
            "my_project",
            "my.project",
            "my/project",
            "hello@world",
        ] {
            let err = validate_alias(bad).unwrap_err();
            assert!(
                err.to_string().contains("invalid character"),
                "expected 'invalid character' for '{}', got: {}",
                bad,
                err
            );
        }
    }

    // ── remove + list ─────────────────────────────────────────────────────

    #[test]
    fn remove_and_list() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "proj");

        reg.add("proj", &dir).unwrap();
        assert_eq!(reg.list().len(), 1);

        reg.remove("proj").unwrap();
        assert_eq!(reg.list().len(), 0);
    }

    #[test]
    fn remove_nonexistent_alias_returns_error() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);

        let err = reg.remove("ghost").unwrap_err();
        assert!(
            err.to_string().contains("not registered"),
            "unexpected error: {}",
            err
        );
    }

    // ── get existing and missing ──────────────────────────────────────────

    #[test]
    fn get_existing_returns_entry() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "proj");

        reg.add("proj", &dir).unwrap();

        let entry = reg.get("proj").expect("should find entry");
        assert_eq!(entry.alias, "proj");
        assert_eq!(entry.path, dir);
    }

    #[test]
    fn get_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);

        assert!(reg.get("nonexistent").is_none());
    }

    // ── path canonicalization and nonexistent path ─────────────────────────

    #[test]
    fn add_nonexistent_path_returns_error() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let bad_path = tmp.path().join("does-not-exist");

        let err = reg.add("bad", &bad_path).unwrap_err();
        assert!(
            err.to_string().contains("does not exist"),
            "unexpected error: {}",
            err
        );
    }

    // ── atomic write: file exists after add ───────────────────────────────

    #[test]
    fn toml_file_exists_after_add() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "proj");

        let toml_path = tmp.path().join(".code-graph").join("projects.toml");
        assert!(!toml_path.exists(), "file should not exist before add");

        reg.add("proj", &dir).unwrap();

        assert!(toml_path.exists(), "file should exist after add");

        // Verify the file is human-readable TOML.
        let content = std::fs::read_to_string(&toml_path).unwrap();
        assert!(
            content.contains("[projects.proj]"),
            "TOML should contain [projects.proj], got:\n{}",
            content
        );
        assert!(
            content.contains("added_at"),
            "TOML should contain added_at field"
        );
    }

    // ── tmp file is cleaned up after atomic write ─────────────────────────

    #[test]
    fn tmp_file_does_not_linger() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "proj");

        reg.add("proj", &dir).unwrap();

        let tmp_path = tmp.path().join(".code-graph").join("projects.toml.tmp");
        assert!(
            !tmp_path.exists(),
            "temporary file should not exist after successful write"
        );
    }

    // ── directory auto-creation ───────────────────────────────────────────

    #[test]
    fn creates_code_graph_directory_automatically() {
        let tmp = TempDir::new().unwrap();
        let cg_dir = tmp.path().join(".code-graph");
        assert!(!cg_dir.exists(), "dir should not pre-exist");

        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "proj");
        reg.add("proj", &dir).unwrap();

        assert!(cg_dir.exists(), "dir should be created automatically");
    }

    // ── multiple projects ─────────────────────────────────────────────────

    #[test]
    fn multiple_projects_are_persisted() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir_a = create_project_dir(&tmp, "proj-a");
        let dir_b = create_project_dir(&tmp, "proj-b");

        reg.add("alpha", &dir_a).unwrap();
        reg.add("beta", &dir_b).unwrap();

        let entries = reg.list();
        assert_eq!(entries.len(), 2);
        // BTreeMap ensures sorted order.
        assert_eq!(entries[0].alias, "alpha");
        assert_eq!(entries[1].alias, "beta");
    }

    // ── integration: add, list, show, remove round-trip ─────────────────

    #[test]
    fn integration_add_list_show_remove() {
        let tmp = TempDir::new().unwrap();
        let reg = test_registry(&tmp);
        let dir = create_project_dir(&tmp, "my-app");

        // Add
        let entry = reg.add("my-app", &dir).unwrap();
        assert_eq!(entry.alias, "my-app");

        // List includes it
        let entries = reg.list();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].alias, "my-app");

        // Show returns it
        let found = reg.get("my-app").unwrap();
        assert_eq!(found.path, dir);

        // Remove
        reg.remove("my-app").unwrap();

        // List is empty
        assert!(reg.list().is_empty());

        // Get returns None
        assert!(reg.get("my-app").is_none());
    }
}
