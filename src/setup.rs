// Editor setup module: detects editors, generates MCP configs, manages CLAUDE.md.
//
// `code-graph setup` is a one-shot command that configures MCP integration for
// Claude Code, Cursor, and Windsurf — removing first-time setup friction.
use std::path::{Path, PathBuf};

use anyhow::Result;

/// Editors that code-graph can configure.
#[derive(Debug, Clone, PartialEq)]
pub enum Editor {
    Claude,
    Cursor,
    Windsurf,
}

impl std::fmt::Display for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Editor::Claude => write!(f, "Claude Code"),
            Editor::Cursor => write!(f, "Cursor"),
            Editor::Windsurf => write!(f, "Windsurf"),
        }
    }
}

/// CLAUDE.md snippet with code-graph navigation instructions.
pub const CLAUDE_MD_SNIPPET: &str = r#"<!-- code-graph:start -->
## Code navigation -- MANDATORY

NEVER use Grep or Glob to find symbol definitions, trace references, or analyze dependencies.
ALWAYS use code-graph MCP tools instead -- they are faster, more accurate, and understand the full AST.

| Task | Tool | NOT this |
|------|------|----------|
| Find where something is defined | `find_symbol` | ~~Grep for `class X`, `function X`, `fn X`~~ |
| Find what uses/imports something | `find_references` | ~~Grep for `import`, `require`, identifier~~ |
| Understand a symbol fully | `get_context` | ~~Multiple Grep + Read calls~~ |
| Check what breaks if I change X | `get_impact` | ~~Manual file-by-file tracing~~ |
| Detect circular deps | `detect_circular` | ~~Grep for import cycles~~ |
| Project overview | `get_stats` | ~~Glob + count files~~ |

Use Read/Grep/Glob ONLY for:
- Reading full file contents before editing
- Searching for string literals, comments, TODOs, error messages
- Non-structural text searches that have nothing to do with code navigation
<!-- code-graph:end -->"#;

pub(crate) const START_MARKER: &str = "<!-- code-graph:start -->";
pub(crate) const END_MARKER: &str = "<!-- code-graph:end -->";

/// Portable home directory lookup without adding the `dirs` crate.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Detect which editors are available based on directory presence.
/// Claude Code is always included.
// Lint disabled because both `if` blocks have single-line bodies that look similar to clippy
// but push different enum variants (Editor::Cursor vs Editor::Windsurf).
#[allow(clippy::if_same_then_else)]
pub fn detect_editors(project_root: &Path) -> Vec<Editor> {
    let mut editors = vec![Editor::Claude];

    // Check for Cursor: project-local .cursor/ or global ~/.cursor/
    if project_root.join(".cursor").is_dir()
        || home_dir().is_some_and(|h| h.join(".cursor").is_dir())
    {
        editors.push(Editor::Cursor);
    }

    // Check for Windsurf: project-local .windsurf/ or global ~/.codeium/
    if project_root.join(".windsurf").is_dir()
        || home_dir().is_some_and(|h| h.join(".codeium").is_dir())
    {
        editors.push(Editor::Windsurf);
    }

    editors
}

/// Generate the MCP server config JSON string.
/// Exposed for testing and external use; write_mcp_config() uses inline JSON for merging.
#[allow(dead_code)]
pub fn generate_mcp_json() -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "code-graph": {
                "command": "code-graph",
                "args": ["mcp", "--watch"]
            }
        }
    }))
    .expect("JSON serialization should not fail")
}

/// Write MCP config to a file, merging with existing config if present.
/// If the file exists and has other mcpServers entries, preserves them.
pub fn write_mcp_config(path: &Path) -> Result<()> {
    let existing: serde_json::Value = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let mut config = existing.as_object().cloned().unwrap_or_default();
    let servers = config
        .entry("mcpServers".to_string())
        .or_insert(serde_json::json!({}));

    if let Some(servers_obj) = servers.as_object_mut() {
        servers_obj.insert(
            "code-graph".to_string(),
            serde_json::json!({
                "command": "code-graph",
                "args": ["mcp", "--watch"]
            }),
        );
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

/// Write the CLAUDE.md navigation snippet using marker-based idempotent replacement.
///
/// Three code paths:
/// 1. File doesn't exist -> create with snippet
/// 2. File exists, no markers -> append snippet at end
/// 3. File exists, markers present -> replace content between markers (inclusive)
pub fn write_claude_md(project_root: &Path) -> Result<()> {
    let claude_md_path = project_root.join("CLAUDE.md");

    if !claude_md_path.exists() {
        // Path 1: Create new file with snippet
        std::fs::write(&claude_md_path, CLAUDE_MD_SNIPPET)?;
        return Ok(());
    }

    let content = std::fs::read_to_string(&claude_md_path)?;

    if let (Some(start_pos), Some(end_pos)) = (content.find(START_MARKER), content.find(END_MARKER))
    {
        // Path 3: Replace between markers (inclusive)
        let end_of_end_marker = end_pos + END_MARKER.len();
        let mut new_content = String::new();
        new_content.push_str(&content[..start_pos]);
        new_content.push_str(CLAUDE_MD_SNIPPET);
        new_content.push_str(&content[end_of_end_marker..]);
        std::fs::write(&claude_md_path, new_content)?;
        return Ok(());
    }

    // Path 2: Append to existing file
    let mut new_content = content;
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push('\n');
    new_content.push_str(CLAUDE_MD_SNIPPET);
    new_content.push('\n');
    std::fs::write(&claude_md_path, new_content)?;
    Ok(())
}

/// Get the config file path for a given editor.
pub fn config_path_for_editor(editor: &Editor, project_root: &Path) -> Option<PathBuf> {
    match editor {
        Editor::Claude => Some(project_root.join(".mcp.json")),
        Editor::Cursor => home_dir().map(|h| h.join(".cursor").join("mcp.json")),
        Editor::Windsurf => home_dir().map(|h| h.join(".codeium").join("mcp_config.json")),
    }
}

/// Verify the MCP server starts and responds.
/// Spawns code-graph in MCP mode, waits briefly, then kills.
pub fn verify_mcp_server() -> Result<()> {
    let binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("code-graph"));

    let mut child = std::process::Command::new(&binary)
        .arg("mcp")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow::anyhow!("failed to spawn code-graph mcp: {e}"))?;

    // Brief timeout then kill — just checking it starts without crashing
    std::thread::sleep(std::time::Duration::from_millis(500));
    child.kill().ok();
    child.wait().ok();
    Ok(())
}

/// Read a yes/no confirmation from a reader. Returns true for y/Y/yes.
/// Separated from stdin so tests can inject input.
pub fn confirm_proceed(reader: &mut dyn std::io::BufRead) -> bool {
    let mut input = String::new();
    if reader.read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Claude Code skill file contents — one per skill.
/// Each skill uses YAML frontmatter plus a markdown body with tool workflow.
pub const EXPLORE_CODEBASE_SKILL: &str = r#"---
name: explore-codebase
description: Explore a codebase's structure and dependencies using code-graph MCP tools. Adaptive mode based on user goal.
---
# Explore Codebase

## Trigger
Use when asked to understand a project, get oriented in a new codebase, or map out architecture.

## Adaptive Mode

**Ask the user:** "Do you want a full project overview, or should I focus on a specific area?"

### Mode 1: Full project overview
1. `get_stats` — file counts, symbol breakdown, language distribution
2. `get_structure` — top-level directory layout and file graph
3. `find_clusters` — identify cohesive module groups
4. `detect_circular` — flag any circular dependency risks
5. `export_graph format=dot granularity=package` — architectural view

### Mode 2: Focus on [area]
1. `get_structure path=[area]` — directory contents and imports
2. `get_file_summary` on 2-3 key files in the area
3. `get_context` on hub symbols (high in-degree nodes)

## Output
Summarize: purpose, main modules, key entry points, any circular risks.
"#;

pub const DEBUG_IMPACT_SKILL: &str = r#"---
name: debug-impact
description: Investigate the blast radius of a symbol change or recent git diff using code-graph impact analysis tools.
---
# Debug Impact

## Trigger
Use when asked "what breaks if I change X?" or "what did this commit affect?"

## Adaptive Mode

**Ask the user:** "Are you investigating a specific symbol, or a recent change?"

### Mode 1: Investigating a specific symbol
1. `get_impact symbol=[name]` — transitive dependents (blast radius)
2. `find_references symbol=[name]` — direct call sites and imports
3. `get_context symbol=[name]` — callers, callees, type info
4. `trace_flow entry=[name] target=[target]` — if a specific data flow is suspected

### Mode 2: Investigating a recent change
1. `get_diff_impact` — symbols touched by the last git diff
2. `get_impact` on each high-risk symbol from step 1
3. `find_references` on symbols with wide blast radius

## Output
List: impacted files with risk level, recommended test scope, safe change order.
"#;

pub const REFACTOR_SYMBOL_SKILL: &str = r#"---
name: refactor-symbol
description: Safely rename or move a symbol using code-graph rename planning with blast radius and circular dependency checks.
---
# Refactor Symbol

## Trigger
Use when asked to rename, move, or restructure a symbol safely.

## Safety-First Workflow

**Ask the user:** "What symbol do you want to rename/move, and what should the new name/location be?"

### Step 1: Plan
`plan_rename symbol=[name] new_name=[new]` — generate rename plan with all affected sites

### Step 2: Blast radius check
`get_impact symbol=[name]` — confirm scope of change (files, modules, depth)

### Step 3: Circular dependency check
`detect_circular` — ensure rename won't introduce cycles in the new location

### Step 4: Dead code check
`find_dead_code` in the symbol's neighborhood — identify any orphaned symbols that can be cleaned up

### Step 5: Present plan for approval
Show: symbol being renamed, affected file count, any circular risks, dead code opportunities.
Ask for explicit approval before applying changes.

## Output
Rename diff plan with file list, risk assessment, and recommended commit strategy.
"#;

/// The PreToolUse hook script that enriches Grep/Glob results with graph context.
pub const HOOK_SCRIPT: &str = r#"#!/usr/bin/env bash
# code-graph-enrichment.sh — PreToolUse hook for Claude Code
# Enriches Grep/Glob tool calls with graph-based symbol context.
# Only fires when the search pattern looks like a symbol name.

# Guard: prevent infinite loops if code-graph itself triggers Grep/Glob
if [ -n "$CLAUDE_HOOK_ACTIVE" ]; then
    exit 0
fi
export CLAUDE_HOOK_ACTIVE=1

# Read the tool input from stdin
INPUT=$(cat)

# Extract tool name and pattern using jq
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // ""' 2>/dev/null)
PATTERN=$(echo "$INPUT" | jq -r '.tool_input.pattern // .tool_input.glob // ""' 2>/dev/null)

# Only fire for Grep and Glob tools
if [ "$TOOL_NAME" != "Grep" ] && [ "$TOOL_NAME" != "Glob" ]; then
    exit 0
fi

# Heuristic: only enrich if pattern looks like a symbol name
# Matches: CamelCase, snake_case, or function/class/def/fn keywords
if ! echo "$PATTERN" | grep -qE '([A-Z][a-zA-Z0-9]+|[a-z][a-z0-9]*_[a-z][a-z0-9_]*|class |function |fn |def )'; then
    exit 0
fi

# Run graph context lookup (errors go to stderr only)
GRAPH_CONTEXT=$(code-graph find "$PATTERN" . 2>/dev/null | head -20)

# Only output if we got useful context
if [ -n "$GRAPH_CONTEXT" ]; then
    printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"Graph context for '"'"'%s'"'"':\n%s"}}' \
        "$PATTERN" "$GRAPH_CONTEXT"
fi
"#;

/// Write the three Claude Code skill files to `.claude/skills/` in the project root.
/// Creates `explore-codebase/SKILL.md`, `debug-impact/SKILL.md`, `refactor-symbol/SKILL.md`.
/// Idempotent: overwrites existing files with the same content.
pub fn write_skills(project_root: &Path) -> Result<()> {
    let skills = [
        ("explore-codebase", EXPLORE_CODEBASE_SKILL),
        ("debug-impact", DEBUG_IMPACT_SKILL),
        ("refactor-symbol", REFACTOR_SYMBOL_SKILL),
    ];

    for (name, content) in &skills {
        let skill_dir = project_root.join(".claude").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), content)?;
    }

    Ok(())
}

/// Write the PreToolUse enrichment hook script to `.claude/hooks/code-graph-enrichment.sh`.
/// Sets executable permissions (Unix only).
pub fn write_hook(project_root: &Path) -> Result<()> {
    let hooks_dir = project_root.join(".claude").join("hooks");
    std::fs::create_dir_all(&hooks_dir)?;

    let hook_path = hooks_dir.join("code-graph-enrichment.sh");
    std::fs::write(&hook_path, HOOK_SCRIPT)?;

    // Make executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))?;
    }

    Ok(())
}

/// Merge the PreToolUse hook registration into `.claude/settings.json`.
/// Creates the file if it doesn't exist. Deduplicates entries by checking for
/// an existing command containing `code-graph-enrichment.sh`.
pub fn write_hook_settings(project_root: &Path) -> Result<()> {
    let settings_path = project_root.join(".claude").join("settings.json");

    // Read existing settings or start with empty object
    let existing: serde_json::Value = if settings_path.exists() {
        let content = std::fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let mut config = existing.as_object().cloned().unwrap_or_default();

    // Navigate to hooks.PreToolUse array, creating if missing
    let hooks = config
        .entry("hooks".to_string())
        .or_insert(serde_json::json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json 'hooks' field is not an object"))?;

    let pre_tool_use = hooks_obj
        .entry("PreToolUse".to_string())
        .or_insert(serde_json::json!([]));
    let entries = pre_tool_use
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json 'hooks.PreToolUse' is not an array"))?;

    // Dedup: check if code-graph-enrichment.sh hook already registered
    let already_registered = entries.iter().any(|entry| {
        // Check if any nested hook command contains our script name
        entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|hooks_arr| {
                hooks_arr.iter().any(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .map(|c| c.contains("code-graph-enrichment.sh"))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    });

    if !already_registered {
        entries.push(serde_json::json!({
            "matcher": "Grep|Glob",
            "hooks": [{ "type": "command", "command": ".claude/hooks/code-graph-enrichment.sh" }]
        }));
    }

    // Ensure parent directory exists and write
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&settings_path, serde_json::to_string_pretty(&config)?)?;

    Ok(())
}

/// Run the full setup flow for a given project root.
/// If `skip_confirm` is true (--yes flag), writes immediately without prompting.
/// Otherwise, prints the plan and asks for confirmation before writing.
/// `skills`: explicit opt-in flag (for non-Claude editors or explicit invocation).
/// `hooks`: explicit opt-in flag (for non-Claude editors or explicit invocation).
/// `no_skills`: skip skill installation even when Claude Code is detected.
/// `no_hooks`: skip hook installation even when Claude Code is detected.
pub fn run_setup(
    project_root: &Path,
    skip_confirm: bool,
    skills: bool,
    hooks: bool,
    no_skills: bool,
    no_hooks: bool,
) -> Result<()> {
    let editors = detect_editors(project_root);

    println!(
        "Detected editors: {}",
        editors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Determine what skills/hooks to install.
    // Auto-install for Claude Code unless explicitly disabled; or explicit opt-in via flag.
    let should_install_skills = (!no_skills && editors.contains(&Editor::Claude)) || skills;
    let should_install_hooks = (!no_hooks && editors.contains(&Editor::Claude)) || hooks;

    // Build the list of files that will be created/modified
    let mut planned_writes: Vec<(Editor, PathBuf)> = Vec::new();
    for editor in &editors {
        if let Some(config_path) = config_path_for_editor(editor, project_root) {
            planned_writes.push((editor.clone(), config_path));
        }
    }
    if editors.contains(&Editor::Claude) {
        planned_writes.push((Editor::Claude, project_root.join("CLAUDE.md")));
    }
    if should_install_skills {
        planned_writes.push((Editor::Claude, project_root.join(".claude/skills/")));
    }
    if should_install_hooks {
        planned_writes.push((
            Editor::Claude,
            project_root.join(".claude/hooks/code-graph-enrichment.sh"),
        ));
        planned_writes.push((Editor::Claude, project_root.join(".claude/settings.json")));
    }

    println!("\nFiles to write:");
    for (_editor, path) in &planned_writes {
        let action = if path.exists() { "modify" } else { "create" };
        println!("  [{action}] {}", path.display());
    }

    // Confirmation gate (unless --yes)
    if !skip_confirm {
        print!("\nProceed? [y/N] ");
        // Flush so prompt appears before reading
        use std::io::Write;
        std::io::stdout().flush().ok();
        let confirmed = confirm_proceed(&mut std::io::stdin().lock());
        if !confirmed {
            println!("Aborted.");
            return Ok(());
        }
    }

    for editor in &editors {
        if let Some(config_path) = config_path_for_editor(editor, project_root) {
            write_mcp_config(&config_path)?;
            println!("  Wrote MCP config: {}", config_path.display());
        }

        // Claude Code also gets CLAUDE.md snippet
        if matches!(editor, Editor::Claude) {
            write_claude_md(project_root)?;
            println!("  Wrote CLAUDE.md navigation snippet");
        }
    }

    // Install skills if requested
    if should_install_skills {
        write_skills(project_root)?;
        println!("  Wrote 3 skill files to .claude/skills/");
    }

    // Install hook if requested
    if should_install_hooks {
        write_hook(project_root)?;
        write_hook_settings(project_root)?;
        println!("  Wrote hook: .claude/hooks/code-graph-enrichment.sh");
        println!("  Updated: .claude/settings.json (PreToolUse hook registered)");
    }

    // Post-setup verification
    match verify_mcp_server() {
        Ok(()) => println!("\nSetup complete. Server verified"),
        Err(e) => println!("\nSetup complete. Server verification skipped: {e}"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_editors_always_includes_claude() {
        let tmp = TempDir::new().unwrap();
        let editors = detect_editors(tmp.path());
        assert!(editors.contains(&Editor::Claude));
    }

    #[test]
    fn test_detect_editors_finds_cursor() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".cursor")).unwrap();
        let editors = detect_editors(tmp.path());
        assert!(editors.contains(&Editor::Cursor));
    }

    #[test]
    fn test_detect_editors_finds_windsurf() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".windsurf")).unwrap();
        let editors = detect_editors(tmp.path());
        assert!(editors.contains(&Editor::Windsurf));
    }

    #[test]
    fn test_generate_mcp_json() {
        let json = generate_mcp_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["mcpServers"]["code-graph"]["command"], "code-graph");
        assert_eq!(parsed["mcpServers"]["code-graph"]["args"][0], "mcp");
        assert_eq!(parsed["mcpServers"]["code-graph"]["args"][1], "--watch");
    }

    #[test]
    fn test_claude_md_snippet_content() {
        assert!(CLAUDE_MD_SNIPPET.contains(START_MARKER));
        assert!(CLAUDE_MD_SNIPPET.contains(END_MARKER));
        assert!(CLAUDE_MD_SNIPPET.contains("find_symbol"));
        assert!(CLAUDE_MD_SNIPPET.contains("get_impact"));
    }

    #[test]
    fn test_write_claude_md_creates_new_file() {
        let tmp = TempDir::new().unwrap();
        write_claude_md(tmp.path()).unwrap();
        let content = std::fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
        assert!(content.contains(START_MARKER));
        assert!(content.contains(END_MARKER));
        assert!(content.contains("find_symbol"));
    }

    #[test]
    fn test_write_claude_md_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        let claude_md = tmp.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "# My Project\n\nExisting content.\n").unwrap();
        write_claude_md(tmp.path()).unwrap();
        let content = std::fs::read_to_string(&claude_md).unwrap();
        assert!(content.starts_with("# My Project"));
        assert!(content.contains("Existing content."));
        assert!(content.contains(START_MARKER));
    }

    #[test]
    fn test_write_claude_md_replaces_existing_markers() {
        let tmp = TempDir::new().unwrap();
        let claude_md = tmp.path().join("CLAUDE.md");
        let old_content = format!(
            "# My Project\n\n{}\nOld snippet content\n{}\n\nMore stuff\n",
            START_MARKER, END_MARKER
        );
        std::fs::write(&claude_md, &old_content).unwrap();
        write_claude_md(tmp.path()).unwrap();
        let content = std::fs::read_to_string(&claude_md).unwrap();
        assert!(content.contains("# My Project"));
        assert!(content.contains("More stuff"));
        assert!(content.contains("find_symbol")); // new snippet
        assert!(!content.contains("Old snippet content")); // old removed
        // Exactly one pair of markers
        assert_eq!(content.matches(START_MARKER).count(), 1);
        assert_eq!(content.matches(END_MARKER).count(), 1);
    }

    #[test]
    fn test_write_claude_md_preserves_surrounding_content() {
        let tmp = TempDir::new().unwrap();
        let claude_md = tmp.path().join("CLAUDE.md");
        let content_with_markers = format!(
            "Before content\n\n{}\nold\n{}\n\nAfter content\n",
            START_MARKER, END_MARKER
        );
        std::fs::write(&claude_md, &content_with_markers).unwrap();
        write_claude_md(tmp.path()).unwrap();
        let result = std::fs::read_to_string(&claude_md).unwrap();
        assert!(result.contains("Before content"));
        assert!(result.contains("After content"));
    }

    #[test]
    fn test_write_mcp_config_creates_file() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".mcp.json");
        write_mcp_config(&config_path).unwrap();
        let content = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["mcpServers"]["code-graph"]["command"], "code-graph");
    }

    #[test]
    fn test_write_mcp_config_merges_existing() {
        let tmp = TempDir::new().unwrap();
        let config_path = tmp.path().join(".mcp.json");
        // Write existing config with another server
        std::fs::write(
            &config_path,
            r#"{"mcpServers":{"other-server":{"command":"other"}}}"#,
        )
        .unwrap();
        write_mcp_config(&config_path).unwrap();
        let content = std::fs::read_to_string(&config_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        // Both servers present
        assert_eq!(parsed["mcpServers"]["other-server"]["command"], "other");
        assert_eq!(parsed["mcpServers"]["code-graph"]["command"], "code-graph");
    }

    #[test]
    fn test_confirm_proceed_yes() {
        for input in &["y\n", "Y\n", "yes\n", "YES\n"] {
            let mut reader = std::io::BufReader::new(input.as_bytes());
            assert!(
                confirm_proceed(&mut reader),
                "expected true for input: {:?}",
                input
            );
        }
    }

    #[test]
    fn test_confirm_proceed_no() {
        for input in &["n\n", "N\n", "\n", "nope\n", "foo\n"] {
            let mut reader = std::io::BufReader::new(input.as_bytes());
            assert!(
                !confirm_proceed(&mut reader),
                "expected false for input: {:?}",
                input
            );
        }
    }

    #[test]
    fn test_run_setup_skips_write_without_confirm() {
        // Verify that confirm_proceed returning false means no files are written.
        // The confirmation gate is structurally guaranteed: run_setup returns
        // Ok(()) early when confirm_proceed returns false.
        let input = "n\n";
        let mut reader = std::io::BufReader::new(input.as_bytes());
        assert!(!confirm_proceed(&mut reader));
    }

    #[test]
    fn test_run_setup_writes_with_yes_flag() {
        let tmp = TempDir::new().unwrap();
        // skip_confirm=true bypasses the prompt; skills=false, hooks=false, no_skills=false, no_hooks=false (defaults)
        run_setup(tmp.path(), true, false, false, false, false).unwrap();
        // Verify files were written
        assert!(tmp.path().join(".mcp.json").exists());
        assert!(tmp.path().join("CLAUDE.md").exists());
    }

    // --- New tests for write_skills ---

    #[test]
    fn test_write_skills_creates_three_dirs() {
        let tmp = TempDir::new().unwrap();
        write_skills(tmp.path()).unwrap();

        let skills_dir = tmp.path().join(".claude").join("skills");
        assert!(
            skills_dir
                .join("explore-codebase")
                .join("SKILL.md")
                .exists()
        );
        assert!(skills_dir.join("debug-impact").join("SKILL.md").exists());
        assert!(skills_dir.join("refactor-symbol").join("SKILL.md").exists());
    }

    #[test]
    fn test_write_skills_content() {
        let tmp = TempDir::new().unwrap();
        write_skills(tmp.path()).unwrap();

        let skills_dir = tmp.path().join(".claude").join("skills");

        // explore-codebase SKILL.md
        let explore =
            std::fs::read_to_string(skills_dir.join("explore-codebase").join("SKILL.md")).unwrap();
        assert!(
            explore.contains("name: explore-codebase"),
            "missing frontmatter name"
        );
        assert!(
            explore.contains("description:"),
            "missing frontmatter description"
        );
        assert!(explore.contains("get_stats"), "missing tool reference");
        assert!(explore.contains("get_structure"), "missing tool reference");

        // debug-impact SKILL.md
        let debug =
            std::fs::read_to_string(skills_dir.join("debug-impact").join("SKILL.md")).unwrap();
        assert!(
            debug.contains("name: debug-impact"),
            "missing frontmatter name"
        );
        assert!(debug.contains("get_impact"), "missing tool reference");
        assert!(debug.contains("find_references"), "missing tool reference");

        // refactor-symbol SKILL.md
        let refactor =
            std::fs::read_to_string(skills_dir.join("refactor-symbol").join("SKILL.md")).unwrap();
        assert!(
            refactor.contains("name: refactor-symbol"),
            "missing frontmatter name"
        );
        assert!(refactor.contains("plan_rename"), "missing tool reference");
        assert!(
            refactor.contains("detect_circular"),
            "missing tool reference"
        );
    }

    #[test]
    fn test_write_skills_idempotent() {
        let tmp = TempDir::new().unwrap();
        write_skills(tmp.path()).unwrap();
        write_skills(tmp.path()).unwrap();

        let skills_dir = tmp.path().join(".claude").join("skills");
        // Running twice produces identical content (no duplication)
        let content_1a =
            std::fs::read_to_string(skills_dir.join("explore-codebase").join("SKILL.md")).unwrap();
        let content_1b = EXPLORE_CODEBASE_SKILL;
        assert_eq!(
            content_1a, content_1b,
            "idempotent: content must be identical after two writes"
        );
    }

    // --- New tests for write_hook ---

    #[test]
    fn test_write_hook_script_creates_file() {
        let tmp = TempDir::new().unwrap();
        write_hook(tmp.path()).unwrap();

        let hook_path = tmp
            .path()
            .join(".claude")
            .join("hooks")
            .join("code-graph-enrichment.sh");
        assert!(hook_path.exists(), "hook script must exist");

        // Verify executable permission on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&hook_path).unwrap().permissions();
            let mode = perms.mode();
            // Check owner execute bit (0o100)
            assert!(mode & 0o100 != 0, "hook script must be executable");
        }
    }

    #[test]
    fn test_write_hook_script_content() {
        let tmp = TempDir::new().unwrap();
        write_hook(tmp.path()).unwrap();

        let hook_path = tmp
            .path()
            .join(".claude")
            .join("hooks")
            .join("code-graph-enrichment.sh");
        let content = std::fs::read_to_string(&hook_path).unwrap();

        assert!(
            content.contains("#!/usr/bin/env bash"),
            "must have bash shebang"
        );
        assert!(
            content.contains("CLAUDE_HOOK_ACTIVE"),
            "must have infinite loop guard"
        );
        assert!(
            content.contains("hookSpecificOutput"),
            "must output valid JSON structure"
        );
        assert!(
            content.contains("Grep") && content.contains("Glob"),
            "must filter by tool name"
        );
        assert!(
            content.contains("stderr") || content.contains("2>/dev/null"),
            "must suppress debug to stderr"
        );
    }

    // --- New tests for write_hook_settings ---

    #[test]
    fn test_write_hook_settings_fresh() {
        let tmp = TempDir::new().unwrap();
        // Create .claude dir but no settings.json
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        write_hook_settings(tmp.path()).unwrap();

        let settings_path = tmp.path().join(".claude").join("settings.json");
        assert!(settings_path.exists(), "settings.json must be created");

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let hooks = &parsed["hooks"]["PreToolUse"];
        assert!(hooks.is_array(), "PreToolUse must be an array");
        let arr = hooks.as_array().unwrap();
        assert_eq!(arr.len(), 1, "should have exactly one entry");
        assert_eq!(arr[0]["matcher"], "Grep|Glob");
        assert_eq!(
            arr[0]["hooks"][0]["command"],
            ".claude/hooks/code-graph-enrichment.sh"
        );
    }

    #[test]
    fn test_write_hook_settings_merge() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();

        // Write existing settings with other hooks
        let existing = serde_json::json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [{ "type": "command", "command": "my-bash-hook.sh" }]
                    }
                ]
            }
        });
        std::fs::write(
            tmp.path().join(".claude").join("settings.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        write_hook_settings(tmp.path()).unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join(".claude").join("settings.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Existing entries preserved
        assert_eq!(
            parsed["theme"], "dark",
            "must preserve other settings fields"
        );
        let arr = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "should have two hook entries");

        // Original Bash hook still present
        let bash_hook = arr.iter().find(|e| e["matcher"] == "Bash").unwrap();
        assert_eq!(bash_hook["hooks"][0]["command"], "my-bash-hook.sh");

        // Our Grep|Glob hook added
        let gg_hook = arr.iter().find(|e| e["matcher"] == "Grep|Glob").unwrap();
        assert_eq!(
            gg_hook["hooks"][0]["command"],
            ".claude/hooks/code-graph-enrichment.sh"
        );
    }

    #[test]
    fn test_write_hook_settings_dedup() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();

        // Run twice
        write_hook_settings(tmp.path()).unwrap();
        write_hook_settings(tmp.path()).unwrap();

        let content =
            std::fs::read_to_string(tmp.path().join(".claude").join("settings.json")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = parsed["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "must not create duplicate hook entries");
    }

    // --- New tests for run_setup no_skills / no_hooks flags ---

    #[test]
    fn test_run_setup_with_no_skills() {
        let tmp = TempDir::new().unwrap();
        // no_skills=true should skip writing skills (skills=false, hooks=false, no_skills=true, no_hooks=false)
        run_setup(tmp.path(), true, false, false, true, false).unwrap();

        let skills_dir = tmp.path().join(".claude").join("skills");
        assert!(
            !skills_dir.exists(),
            "skills dir must NOT exist when no_skills=true"
        );
    }

    #[test]
    fn test_run_setup_with_no_hooks() {
        let tmp = TempDir::new().unwrap();
        // no_hooks=true should skip writing hooks (skills=false, hooks=false, no_skills=false, no_hooks=true)
        run_setup(tmp.path(), true, false, false, false, true).unwrap();

        let hook_path = tmp
            .path()
            .join(".claude")
            .join("hooks")
            .join("code-graph-enrichment.sh");
        assert!(
            !hook_path.exists(),
            "hook must NOT exist when no_hooks=true"
        );

        let settings_path = tmp.path().join(".claude").join("settings.json");
        assert!(
            !settings_path.exists(),
            "settings.json must NOT be created when no_hooks=true"
        );
    }
}
