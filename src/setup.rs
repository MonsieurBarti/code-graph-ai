use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Hook script filenames that code-graph manages.
const HOOK_FILES: &[&str] = &["codegraph-pretool-bash.sh", "codegraph-pretool-search.sh"];

/// The hook matcher entries that code-graph adds to settings.json.
const BASH_HOOK_COMMAND: &str = ".claude/hooks/codegraph-pretool-bash.sh";
const SEARCH_HOOK_COMMAND: &str = ".claude/hooks/codegraph-pretool-search.sh";
const PERMISSION_ENTRY: &str = "Bash(code-graph *)";

/// Run the setup (or uninstall) workflow.
pub fn run(global: bool, uninstall: bool) -> Result<()> {
    let base_dir = resolve_base_dir(global)?;

    if uninstall {
        run_uninstall(&base_dir)?;
    } else {
        run_install(&base_dir, global)?;
    }

    Ok(())
}

/// Determine the target directory for hook installation.
fn resolve_base_dir(global: bool) -> Result<PathBuf> {
    if global {
        let home = std::env::var("HOME").context("HOME environment variable not set")?;
        Ok(PathBuf::from(home).join(".claude"))
    } else {
        // Project-level: use .claude/ relative to cwd
        Ok(PathBuf::from(".claude"))
    }
}

/// Install hooks and configure settings.
fn run_install(base_dir: &Path, global: bool) -> Result<()> {
    let hooks_dir = base_dir.join("hooks");
    let settings_path = base_dir.join("settings.json");

    // Ensure directories exist
    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("Failed to create hooks directory: {}", hooks_dir.display()))?;

    // 1. Copy hook scripts
    let mut hooks_installed = Vec::new();
    for &hook_file in HOOK_FILES {
        let dest = hooks_dir.join(hook_file);
        let source = find_hook_source(hook_file)?;
        fs::copy(&source, &dest).with_context(|| {
            format!("Failed to copy {} to {}", source.display(), dest.display())
        })?;
        set_executable(&dest)?;
        hooks_installed.push(hook_file);
    }

    // 2. Merge hook config into settings.json
    let settings_modified = merge_settings(&settings_path, global)?;

    // 3. Clean up MCP config (project-level only — global has no .mcp.json)
    let mut mcp_actions = Vec::new();
    if !global {
        mcp_actions = cleanup_mcp(base_dir)?;
    }

    // 4. Print summary
    println!("code-graph setup complete!\n");
    println!(
        "  Target: {}",
        if global {
            "global (~/.claude/)"
        } else {
            "project (.claude/)"
        }
    );
    println!("\n  Hooks installed:");
    for hook in &hooks_installed {
        println!("    + {}/hooks/{}", base_dir.display(), hook);
    }
    if settings_modified {
        println!("\n  Settings updated:");
        println!("    ~ {}", settings_path.display());
    }
    if !mcp_actions.is_empty() {
        println!("\n  MCP cleanup:");
        for action in &mcp_actions {
            println!("    - {action}");
        }
    }

    Ok(())
}

/// Uninstall code-graph hooks and permissions.
fn run_uninstall(base_dir: &Path) -> Result<()> {
    let hooks_dir = base_dir.join("hooks");
    let settings_path = base_dir.join("settings.json");

    // 1. Remove hook scripts
    let mut removed = Vec::new();
    for &hook_file in HOOK_FILES {
        let path = hooks_dir.join(hook_file);
        if path.exists() {
            fs::remove_file(&path)?;
            removed.push(hook_file);
        }
    }

    // 2. Remove hook entries from settings.json
    let settings_modified = remove_from_settings(&settings_path)?;

    // 3. Print summary
    println!("code-graph uninstall complete!\n");
    if !removed.is_empty() {
        println!("  Hooks removed:");
        for hook in &removed {
            println!("    - {}/hooks/{}", base_dir.display(), hook);
        }
    }
    if settings_modified {
        println!("\n  Settings updated:");
        println!("    ~ {}", settings_path.display());
    }

    Ok(())
}

/// Find the source path for a hook script.
///
/// When running from the project repo, the scripts are in `.claude/hooks/`.
/// When running as an installed binary, we embed them or look relative to the binary.
fn find_hook_source(hook_file: &str) -> Result<PathBuf> {
    // Try project-local first (running from the code-graph repo itself)
    let local = PathBuf::from(".claude/hooks").join(hook_file);
    if local.exists() {
        return Ok(local);
    }

    // Try next to the binary
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        let beside_binary = exe_dir.join("hooks").join(hook_file);
        if beside_binary.exists() {
            return Ok(beside_binary);
        }
    }

    anyhow::bail!(
        "Hook script '{}' not found. Run setup from the code-graph project directory, \
         or ensure hook scripts are installed alongside the binary.",
        hook_file
    );
}

/// Set the executable bit on a file (Unix).
#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(perms.mode() | 0o111);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Merge code-graph hook entries into settings.json without clobbering existing hooks.
fn merge_settings(settings_path: &Path, global: bool) -> Result<bool> {
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let mut modified = false;

    // Determine command prefix based on scope
    let (bash_cmd, search_cmd) = if global {
        (
            format!(
                "{}/hooks/codegraph-pretool-bash.sh",
                resolve_base_dir(true)?.display()
            ),
            format!(
                "{}/hooks/codegraph-pretool-search.sh",
                resolve_base_dir(true)?.display()
            ),
        )
    } else {
        (
            BASH_HOOK_COMMAND.to_string(),
            SEARCH_HOOK_COMMAND.to_string(),
        )
    };

    // Ensure hooks.PreToolUse exists
    let hooks = settings
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}));
    let pre_tool_use = hooks
        .as_object_mut()
        .unwrap()
        .entry("PreToolUse")
        .or_insert_with(|| serde_json::json!([]));

    let arr = pre_tool_use.as_array_mut().unwrap();

    // Add/update Bash matcher with codegraph hook
    modified |= ensure_hook_entry(arr, "Bash", &bash_cmd);

    // Add/update Grep|Glob matcher with codegraph hook
    modified |= ensure_hook_entry(arr, "Grep|Glob", &search_cmd);

    // Add permission
    let permissions = settings
        .as_object_mut()
        .unwrap()
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    let allow = permissions
        .as_object_mut()
        .unwrap()
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]));

    let allow_arr = allow.as_array_mut().unwrap();
    let perm_str = serde_json::Value::String(PERMISSION_ENTRY.to_string());
    if !allow_arr.contains(&perm_str) {
        allow_arr.push(perm_str);
        modified = true;
    }

    // Remove stale MCP permissions
    let before_len = allow_arr.len();
    allow_arr.retain(|v| {
        v.as_str()
            .is_none_or(|s| !s.starts_with("mcp__code-graph__"))
    });
    if allow_arr.len() != before_len {
        modified = true;
    }

    if modified {
        let content = serde_json::to_string_pretty(&settings)?;
        fs::write(settings_path, content + "\n")?;
    }

    Ok(modified)
}

/// Ensure a hook command exists under the given matcher in the PreToolUse array.
/// Returns true if any modification was made.
fn ensure_hook_entry(
    pre_tool_use: &mut Vec<serde_json::Value>,
    matcher: &str,
    command: &str,
) -> bool {
    let hook_entry = serde_json::json!({
        "type": "command",
        "command": command
    });

    // Find existing matcher group
    for entry in pre_tool_use.iter_mut() {
        if entry.get("matcher").and_then(|m| m.as_str()) == Some(matcher) {
            let hooks = entry
                .as_object_mut()
                .unwrap()
                .entry("hooks")
                .or_insert_with(|| serde_json::json!([]));
            let hooks_arr = hooks.as_array_mut().unwrap();

            // Check if our hook is already there
            let already_has = hooks_arr.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains("codegraph-pretool"))
            });

            if !already_has {
                hooks_arr.push(hook_entry);
                return true;
            }
            return false;
        }
    }

    // No existing matcher group — create one
    pre_tool_use.push(serde_json::json!({
        "matcher": matcher,
        "hooks": [hook_entry]
    }));
    true
}

/// Remove code-graph hook entries from settings.json.
fn remove_from_settings(settings_path: &Path) -> Result<bool> {
    if !settings_path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(settings_path)?;
    let mut settings: serde_json::Value =
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}));

    let mut modified = false;

    // Remove hooks
    if let Some(hooks) = settings.get_mut("hooks")
        && let Some(pre_tool_use) = hooks.get_mut("PreToolUse")
        && let Some(arr) = pre_tool_use.as_array_mut()
    {
        for entry in arr.iter_mut() {
            if let Some(hooks_arr) = entry.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                let before = hooks_arr.len();
                hooks_arr.retain(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .is_none_or(|c| !c.contains("codegraph-pretool"))
                });
                if hooks_arr.len() != before {
                    modified = true;
                }
            }
        }
        // Remove empty matcher groups
        let before = arr.len();
        arr.retain(|entry| {
            entry
                .get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|a| !a.is_empty())
        });
        if arr.len() != before {
            modified = true;
        }
    }

    // Remove permission
    if let Some(permissions) = settings.get_mut("permissions")
        && let Some(allow) = permissions.get_mut("allow")
        && let Some(arr) = allow.as_array_mut()
    {
        let before = arr.len();
        arr.retain(|v| v.as_str() != Some(PERMISSION_ENTRY));
        if arr.len() != before {
            modified = true;
        }
    }

    if modified {
        let content = serde_json::to_string_pretty(&settings)?;
        fs::write(settings_path, content + "\n")?;
    }

    Ok(modified)
}

/// Clean up stale MCP configuration.
fn cleanup_mcp(base_dir: &Path) -> Result<Vec<String>> {
    let mut actions = Vec::new();

    // 1. Clean .mcp.json
    let mcp_path = base_dir
        .parent()
        .unwrap_or(Path::new("."))
        .join(".mcp.json");
    if mcp_path.exists() {
        let content = fs::read_to_string(&mcp_path)?;
        if let Ok(mut mcp) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(servers) = mcp.get_mut("mcpServers").and_then(|s| s.as_object_mut())
            && servers.remove("code-graph").is_some()
        {
            actions.push(format!(
                "Removed 'code-graph' server from {}",
                mcp_path.display()
            ));
            if servers.is_empty() {
                // Remove the file if no servers remain
                fs::remove_file(&mcp_path)?;
                actions.push(format!("Deleted empty {}", mcp_path.display()));
            } else {
                let content = serde_json::to_string_pretty(&mcp)?;
                fs::write(&mcp_path, content + "\n")?;
            }
        }
    }

    // 2. Clean settings.local.json MCP permissions
    let settings_local_path = base_dir.join("settings.local.json");
    if settings_local_path.exists() {
        let content = fs::read_to_string(&settings_local_path)?;
        if let Ok(mut settings) = serde_json::from_str::<serde_json::Value>(&content) {
            let mut local_modified = false;

            // Remove MCP permissions
            if let Some(permissions) = settings.get_mut("permissions")
                && let Some(allow) = permissions.get_mut("allow")
                && let Some(arr) = allow.as_array_mut()
            {
                let before = arr.len();
                arr.retain(|v| {
                    v.as_str()
                        .is_none_or(|s| !s.starts_with("mcp__code-graph__"))
                });
                if arr.len() != before {
                    local_modified = true;
                    actions.push(format!(
                        "Removed {} mcp__code-graph__* permission(s) from {}",
                        before - arr.len(),
                        settings_local_path.display()
                    ));
                }
            }

            // Remove enabledMcpjsonServers code-graph entry
            if let Some(enabled) = settings.get_mut("enabledMcpjsonServers")
                && let Some(arr) = enabled.as_array_mut()
            {
                let before = arr.len();
                arr.retain(|v| v.as_str() != Some("code-graph"));
                if arr.len() != before {
                    local_modified = true;
                    actions.push(format!(
                        "Removed 'code-graph' from enabledMcpjsonServers in {}",
                        settings_local_path.display()
                    ));
                }
            }

            if local_modified {
                let content = serde_json::to_string_pretty(&settings)?;
                fs::write(&settings_local_path, content + "\n")?;
            }
        }
    }

    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::Parser;

    #[test]
    fn test_setup_parses() {
        let cli = Cli::parse_from(["code-graph", "setup"]);
        match cli.command {
            crate::cli::Commands::Setup { global, uninstall } => {
                assert!(!global, "--global should default to false");
                assert!(!uninstall, "--uninstall should default to false");
            }
            _ => panic!("expected Setup command"),
        }
    }

    #[test]
    fn test_setup_global_flag() {
        let cli = Cli::parse_from(["code-graph", "setup", "--global"]);
        match cli.command {
            crate::cli::Commands::Setup { global, uninstall } => {
                assert!(global, "--global should be true");
                assert!(!uninstall, "--uninstall should default to false");
            }
            _ => panic!("expected Setup command"),
        }
    }

    #[test]
    fn test_setup_uninstall_flag() {
        let cli = Cli::parse_from(["code-graph", "setup", "--uninstall"]);
        match cli.command {
            crate::cli::Commands::Setup { global, uninstall } => {
                assert!(!global, "--global should default to false");
                assert!(uninstall, "--uninstall should be true");
            }
            _ => panic!("expected Setup command"),
        }
    }

    #[test]
    fn test_ensure_hook_entry_adds_new_matcher() {
        let mut arr: Vec<serde_json::Value> = vec![];
        let modified = ensure_hook_entry(&mut arr, "Bash", "some/hook.sh");
        assert!(modified);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], "Bash");
    }

    #[test]
    fn test_ensure_hook_entry_appends_to_existing() {
        let mut arr: Vec<serde_json::Value> = vec![serde_json::json!({
            "matcher": "Bash",
            "hooks": [
                { "type": "command", "command": "other-hook.sh" }
            ]
        })];
        let modified = ensure_hook_entry(&mut arr, "Bash", "codegraph-pretool-bash.sh");
        assert!(modified);
        assert_eq!(arr.len(), 1); // Still one matcher group
        let hooks = arr[0]["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), 2); // Two hooks now
    }

    #[test]
    fn test_ensure_hook_entry_idempotent() {
        let mut arr: Vec<serde_json::Value> = vec![serde_json::json!({
            "matcher": "Bash",
            "hooks": [
                { "type": "command", "command": "codegraph-pretool-bash.sh" }
            ]
        })];
        let modified = ensure_hook_entry(&mut arr, "Bash", "codegraph-pretool-bash.sh");
        assert!(!modified); // Already present
        let hooks = arr[0]["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), 1); // Still one hook
    }

    #[test]
    fn test_merge_settings_creates_new() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let modified = merge_settings(&settings_path, false).unwrap();
        assert!(modified);
        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        // Should have hooks and permissions
        assert!(settings.get("hooks").is_some());
        assert!(settings.get("permissions").is_some());
        // Should have our permission
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        assert!(allow.contains(&serde_json::Value::String(PERMISSION_ENTRY.to_string())));
    }

    #[test]
    fn test_merge_settings_preserves_existing() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let existing = serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "rtk-rewrite.sh" }
                        ]
                    }
                ]
            },
            "permissions": {
                "allow": ["Bash(git *)"],
                "deny": []
            }
        });
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();
        merge_settings(&settings_path, false).unwrap();
        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        // RTK hook should still be there
        let bash_hooks = &settings["hooks"]["PreToolUse"][0]["hooks"];
        let has_rtk = bash_hooks.as_array().unwrap().iter().any(|h| {
            h.get("command")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c == "rtk-rewrite.sh")
        });
        assert!(has_rtk, "RTK hook should be preserved");
        // Our hook should be added
        let has_ours = bash_hooks.as_array().unwrap().iter().any(|h| {
            h.get("command")
                .and_then(|c| c.as_str())
                .is_some_and(|c| c.contains("codegraph-pretool"))
        });
        assert!(has_ours, "codegraph hook should be added");
        // Git permission should be preserved
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        assert!(allow.contains(&serde_json::Value::String("Bash(git *)".to_string())));
    }

    #[test]
    fn test_merge_settings_removes_mcp_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let existing = serde_json::json!({
            "permissions": {
                "allow": [
                    "mcp__code-graph__find_symbol",
                    "mcp__code-graph__get_stats",
                    "Bash(git *)"
                ]
            }
        });
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();
        merge_settings(&settings_path, false).unwrap();
        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        // MCP permissions should be gone
        assert!(
            !allow.iter().any(|v| v
                .as_str()
                .is_some_and(|s| s.starts_with("mcp__code-graph__"))),
            "MCP permissions should be removed"
        );
        // Non-MCP permissions should remain
        assert!(allow.contains(&serde_json::Value::String("Bash(git *)".to_string())));
    }

    #[test]
    fn test_remove_from_settings() {
        let dir = tempfile::tempdir().unwrap();
        let settings_path = dir.path().join("settings.json");
        let existing = serde_json::json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "rtk-rewrite.sh" },
                            { "type": "command", "command": ".claude/hooks/codegraph-pretool-bash.sh" }
                        ]
                    },
                    {
                        "matcher": "Grep|Glob",
                        "hooks": [
                            { "type": "command", "command": ".claude/hooks/codegraph-pretool-search.sh" }
                        ]
                    }
                ]
            },
            "permissions": {
                "allow": ["Bash(code-graph *)", "Bash(git *)"]
            }
        });
        fs::write(
            &settings_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();
        let modified = remove_from_settings(&settings_path).unwrap();
        assert!(modified);
        let content = fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        // RTK hook should remain
        let bash_hooks = &settings["hooks"]["PreToolUse"][0]["hooks"];
        assert_eq!(bash_hooks.as_array().unwrap().len(), 1);
        assert_eq!(bash_hooks[0]["command"], "rtk-rewrite.sh");
        // Grep|Glob matcher should be removed (empty after removing our hook)
        let pre_tool_use = settings["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(
            pre_tool_use.len(),
            1,
            "Empty Grep|Glob matcher should be removed"
        );
        // code-graph permission gone, git permission remains
        let allow = settings["permissions"]["allow"].as_array().unwrap();
        assert!(!allow.contains(&serde_json::Value::String(PERMISSION_ENTRY.to_string())));
        assert!(allow.contains(&serde_json::Value::String("Bash(git *)".to_string())));
    }
}
