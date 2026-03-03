/// Authentication state and credential loading for the RAG conversational agent.
///
/// Supports two LLM providers:
/// - **Claude** (Anthropic API) — resolved from `ANTHROPIC_API_KEY` env var or `~/.code-graph/auth.toml`
/// - **Ollama** (local LLM) — configured via `~/.code-graph/auth.toml`
///
/// Credential resolution order (for Claude):
/// 1. `ANTHROPIC_API_KEY` environment variable
/// 2. `[claude].api_key` in `~/.code-graph/auth.toml`
/// 3. `None` if neither is set
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ─── Types ────────────────────────────────────────────────────────────────────

/// Supported LLM provider variants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LlmProvider {
    /// Anthropic Claude API with bearer-token authentication.
    Claude { api_key: String },
    /// Locally-running Ollama server.
    Ollama { host: String, model: String },
}

/// Resolved authentication state — created after credential loading.
#[derive(Debug, Clone)]
pub struct AuthState {
    /// The resolved provider with embedded credentials.
    pub provider: LlmProvider,
}

/// Raw deserialized form of `~/.code-graph/auth.toml`.
///
/// ```toml
/// [claude]
/// api_key = "sk-ant-..."
///
/// [ollama]
/// host = "http://localhost:11434"
/// default_model = "llama3.2"
/// ```
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct AuthConfig {
    /// Claude credentials block.
    pub claude: Option<ClaudeConfig>,
    /// Ollama configuration block.
    pub ollama: Option<OllamaConfig>,
}

/// Credentials for the Claude provider stored in `auth.toml`.
#[derive(Debug, Deserialize, Serialize)]
pub struct ClaudeConfig {
    pub api_key: String,
}

/// Configuration for the Ollama provider stored in `auth.toml`.
#[derive(Debug, Deserialize, Serialize)]
pub struct OllamaConfig {
    pub host: String,
    pub default_model: String,
}

// ─── Path helpers ─────────────────────────────────────────────────────────────

/// Returns the path to the `auth.toml` credential file: `~/.code-graph/auth.toml`.
///
/// Uses `HOME` (Unix) / `USERPROFILE` (Windows) env vars to locate the home directory —
/// no `dirs` crate dependency (consistent with Phase 20 pattern).
pub fn auth_toml_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".code-graph").join("auth.toml")
}

// ─── Config loading ───────────────────────────────────────────────────────────

/// Load `~/.code-graph/auth.toml` and parse it as [`AuthConfig`].
///
/// Returns `Ok(None)` if the file does not exist.
/// Returns `Ok(Some(config))` on success.
/// Returns `Err` on parse errors.
///
/// On Unix, also verifies the file has `0600` permissions (owner read/write only)
/// and emits a warning (not an error) if permissions are too permissive.
pub fn load_auth_config() -> Result<Option<AuthConfig>> {
    let path = auth_toml_path();
    if !path.exists() {
        return Ok(None);
    }

    // Unix: warn if permissions are not 0600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(&path)?;
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o600 {
            eprintln!(
                "warning: {} has permissions {:04o}, expected 0600 — consider running: chmod 0600 {}",
                path.display(),
                mode,
                path.display()
            );
        }
    }

    let content = std::fs::read_to_string(&path)?;
    let config: AuthConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("failed to parse {}: {}", path.display(), e))?;
    Ok(Some(config))
}

// ─── Credential resolution ────────────────────────────────────────────────────

/// Resolve the Anthropic API key, checking `ANTHROPIC_API_KEY` first then `auth.toml`.
///
/// Returns `Some(key)` if a key was found, `None` otherwise.
pub fn resolve_api_key() -> Option<String> {
    // Priority 1: environment variable.
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
        && !key.is_empty()
    {
        return Some(key);
    }

    // Priority 2: auth.toml [claude] section.
    if let Ok(Some(config)) = load_auth_config()
        && let Some(claude) = config.claude
        && !claude.api_key.is_empty()
    {
        return Some(claude.api_key);
    }

    None
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // Helper: create a temporary auth.toml with the given content and return its directory.
    #[allow(dead_code)]
    fn write_temp_auth_toml(content: &str) -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        let toml_path = dir.path().join("auth.toml");
        let mut f = std::fs::File::create(&toml_path).expect("create auth.toml");
        f.write_all(content.as_bytes()).expect("write auth.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&toml_path, std::fs::Permissions::from_mode(0o600))
                .expect("set permissions");
        }
        dir
    }

    #[test]
    fn auth_toml_path_returns_code_graph_auth_toml() {
        let path = auth_toml_path();
        assert!(
            path.ends_with(".code-graph/auth.toml"),
            "expected path ending in .code-graph/auth.toml, got: {}",
            path.display()
        );
    }

    #[test]
    fn resolve_api_key_returns_env_var_when_set() {
        // Set env var and verify it wins over anything else.
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::set_var("ANTHROPIC_API_KEY", "sk-ant-test-env-key") };
        let key = resolve_api_key();
        // Clean up immediately to avoid polluting other tests.
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };
        assert_eq!(key, Some("sk-ant-test-env-key".to_string()));
    }

    #[test]
    fn resolve_api_key_returns_none_when_nothing_set() {
        // Isolate HOME to a clean temp dir so parallel tests that mutate HOME
        // (e.g. load_auth_config_parses_claude_section) cannot leak an auth.toml.
        let tmp = TempDir::new().expect("temp dir");
        let original_home = std::env::var("HOME").ok();
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::set_var("HOME", tmp.path().to_str().unwrap()) };
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

        let key = resolve_api_key();

        // Restore HOME.
        if let Some(h) = original_home {
            unsafe { std::env::set_var("HOME", h) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }

        assert!(
            key.is_none(),
            "expected None when no env var and no auth.toml, got: {:?}",
            key
        );
    }

    #[test]
    fn load_auth_config_returns_none_for_missing_file() {
        let tmp = TempDir::new().expect("temp dir");
        // Override the path by setting HOME to the temp dir (no auth.toml there).
        let original_home = std::env::var("HOME").ok();
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::set_var("HOME", tmp.path().to_str().unwrap()) };

        let result = load_auth_config();

        // Restore HOME.
        if let Some(h) = original_home {
            unsafe { std::env::set_var("HOME", h) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }

        assert!(result.is_ok(), "should return Ok");
        assert!(
            result.unwrap().is_none(),
            "should return None for missing file"
        );
    }

    #[test]
    fn load_auth_config_parses_claude_section() {
        let tmp = TempDir::new().expect("temp dir");
        // auth.toml must live directly at ~/.code-graph/auth.toml
        let cg_dir = tmp.path().join(".code-graph");
        std::fs::create_dir_all(&cg_dir).expect("create .code-graph");
        let toml_path = cg_dir.join("auth.toml");
        std::fs::write(&toml_path, "[claude]\napi_key = \"sk-ant-file-key\"\n")
            .expect("write auth.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&toml_path, std::fs::Permissions::from_mode(0o600))
                .expect("set permissions");
        }

        let original_home = std::env::var("HOME").ok();
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::set_var("HOME", tmp.path().to_str().unwrap()) };

        let result = load_auth_config();

        if let Some(h) = original_home {
            unsafe { std::env::set_var("HOME", h) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }

        let config = result.expect("should succeed").expect("should be Some");
        assert_eq!(config.claude.as_ref().unwrap().api_key, "sk-ant-file-key");
    }

    #[test]
    fn resolve_api_key_falls_back_to_auth_toml() {
        // Ensure env var is absent.
        // SAFETY: single-threaded test binary; no concurrent env access.
        unsafe { std::env::remove_var("ANTHROPIC_API_KEY") };

        let tmp = TempDir::new().expect("temp dir");
        let cg_dir = tmp.path().join(".code-graph");
        std::fs::create_dir_all(&cg_dir).expect("create .code-graph");
        let toml_path = cg_dir.join("auth.toml");
        std::fs::write(&toml_path, "[claude]\napi_key = \"sk-ant-from-file\"\n")
            .expect("write auth.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&toml_path, std::fs::Permissions::from_mode(0o600))
                .expect("set permissions");
        }

        let original_home = std::env::var("HOME").ok();
        unsafe { std::env::set_var("HOME", tmp.path().to_str().unwrap()) };

        let key = resolve_api_key();

        if let Some(h) = original_home {
            unsafe { std::env::set_var("HOME", h) };
        } else {
            unsafe { std::env::remove_var("HOME") };
        }

        assert_eq!(key, Some("sk-ant-from-file".to_string()));
    }

    #[test]
    fn auth_state_holds_provider() {
        let state = AuthState {
            provider: LlmProvider::Claude {
                api_key: "test-key".to_string(),
            },
        };
        assert!(matches!(state.provider, LlmProvider::Claude { .. }));
    }

    #[test]
    fn llm_provider_ollama_fields() {
        let provider = LlmProvider::Ollama {
            host: "http://localhost:11434".to_string(),
            model: "llama3.2".to_string(),
        };
        match provider {
            LlmProvider::Ollama { host, model } => {
                assert_eq!(host, "http://localhost:11434");
                assert_eq!(model, "llama3.2");
            }
            _ => panic!("expected Ollama variant"),
        }
    }
}
