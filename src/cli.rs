use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::export;

/// Action for the `daemon` subcommand.
#[derive(Subcommand, Debug)]
pub enum DaemonAction {
    /// Start the background daemon for the given project.
    Start {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },
    /// Stop the background daemon for the given project.
    Stop {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },
    /// Show daemon status for the given project.
    Status {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },
}

/// Action for the `project` subcommand.
#[derive(Subcommand, Debug)]
pub enum ProjectAction {
    /// Register a project with an alias.
    Add {
        /// Alias for the project (alphanumeric and hyphens, 1-64 chars).
        alias: String,
        /// Path to the project root directory.
        path: PathBuf,
    },
    /// Remove a registered project by alias.
    Remove {
        /// Alias of the project to remove.
        alias: String,
    },
    /// List all registered projects.
    List,
    /// Show details of a registered project.
    Show {
        /// Alias of the project to show.
        alias: String,
    },
}

/// Action for the `snapshot` subcommand.
#[derive(Subcommand, Debug)]
pub enum SnapshotAction {
    /// Create a named snapshot of the current graph state.
    Create {
        /// Snapshot name (alphanumeric, hyphens, underscores only).
        name: String,
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },
    /// List all stored snapshots with creation timestamps.
    List {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },
    /// Delete a named snapshot.
    Delete {
        /// Snapshot name to delete.
        name: String,
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },
}

/// A high-performance code intelligence engine for TypeScript/JavaScript codebases.
///
/// code-graph indexes your codebase into a queryable dependency graph, enabling
/// fast navigation and impact analysis without reading source files.
#[derive(Parser, Debug)]
#[command(
    name = "code-graph",
    version,
    about,
    long_about = None,
    propagate_version = true,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

/// Output format for query results.
#[derive(Clone, Debug, ValueEnum, Default)]
pub enum OutputFormat {
    /// Compact one-line-per-result format, token-optimized for AI agent use (default).
    #[default]
    Compact,
    /// Human-readable columnar table with optional ANSI color when stdout is a terminal.
    Table,
    /// Structured JSON array suitable for programmatic consumption.
    Json,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Index a project directory, discovering and parsing all source files.
    Index {
        /// Path to the project root to index.
        path: PathBuf,

        /// Print each discovered file path during indexing.
        #[arg(short, long)]
        verbose: bool,

        /// Output results as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,

        /// Override language auto-detection. Comma-separated or repeated.
        /// Valid: typescript, javascript, rust (or ts, js, rs).
        /// Example: --language rust  or  --language rust,typescript
        #[arg(long, value_delimiter = ',')]
        language: Vec<String>,

        /// Skip building the vector embedding index (disables RAG agent).
        ///
        /// By default, `code-graph index` builds per-symbol vector embeddings using
        /// fastembed and persists them to `.code-graph/vectors.usearch`. Use this flag
        /// to skip the embedding pass (faster indexing, no model download required).
        /// Only available when compiled with `--features rag`.
        #[cfg(feature = "rag")]
        #[arg(long)]
        no_embeddings: bool,
    },

    /// Find a symbol's definition (file:line location).
    ///
    /// Re-indexes the project before executing the query. Supports regex patterns
    /// (e.g. "User.*Service"), case-insensitive matching, kind filters, and file scoping.
    Find {
        /// Symbol name or regex pattern (e.g. "UserService" or "User.*Service").
        symbol: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Case-insensitive pattern matching.
        #[arg(short = 'i', long)]
        case_insensitive: bool,

        /// Filter by symbol kind (comma-separated: function,class,interface,type,enum,variable,component,method,property).
        #[arg(long, value_delimiter = ',')]
        kind: Vec<String>,

        /// Scope search to a specific file or directory path (relative to project root).
        #[arg(long)]
        file: Option<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter results by language (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// Find all references to a symbol across the codebase.
    ///
    /// Reports files that import the symbol's defining file and call sites (Calls edges).
    Refs {
        /// Symbol name or regex pattern.
        symbol: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Case-insensitive pattern matching.
        #[arg(short = 'i', long)]
        case_insensitive: bool,

        /// Filter by symbol kind (comma-separated).
        #[arg(long, value_delimiter = ',')]
        kind: Vec<String>,

        /// Scope search to a specific file or directory path.
        #[arg(long)]
        file: Option<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter results by language (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// Show the transitive blast radius (dependents) of changing a symbol.
    ///
    /// Performs reverse BFS on the import graph from the symbol's defining file.
    Impact {
        /// Symbol name or regex pattern.
        symbol: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Case-insensitive pattern matching.
        #[arg(short = 'i', long)]
        case_insensitive: bool,

        /// Show hierarchical dependency chain view (default is flat list).
        #[arg(long)]
        tree: bool,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter results by language (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// Detect circular dependencies in the import graph (file-level).
    ///
    /// Uses Kosaraju's SCC algorithm. Each reported cycle is a set of files
    /// that mutually import each other directly or transitively.
    Circular {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter results by language (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// Project statistics overview: file count, symbol breakdown, import summary.
    Stats {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter output to show only a specific language's stats section (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// 360-degree view of a symbol: definition, references, callers, and callees.
    ///
    /// Combines find + refs + call graph edges in a single query pass.
    Context {
        /// Symbol name or regex pattern.
        symbol: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Case-insensitive pattern matching.
        #[arg(short = 'i', long)]
        case_insensitive: bool,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter results by language (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// Start a file watcher that monitors for changes and re-indexes incrementally.
    Watch {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,
    },

    /// Create, list, or delete graph snapshots for diff comparisons.
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },

    /// Start a web server with interactive graph visualization UI.
    #[cfg(feature = "web")]
    Serve {
        /// Path to the project root (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Port to listen on.
        #[arg(long, default_value_t = 7070)]
        port: u16,
        /// Use Ollama as the default LLM provider for the RAG chat agent (offline mode).
        ///
        /// Connects to a locally-running Ollama instance at http://localhost:11434.
        /// The API key prompt in the chat UI will be skipped.
        #[cfg(feature = "rag")]
        #[arg(long)]
        ollama: bool,
    },

    /// Export the code graph to DOT or Mermaid format for architectural visualization.
    Export {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format: dot (default) or mermaid.
        #[arg(long, value_enum, default_value_t = export::model::ExportFormat::Dot)]
        format: export::model::ExportFormat,

        /// Granularity: file (default), symbol, or package.
        #[arg(long, value_enum, default_value_t = export::model::Granularity::File)]
        granularity: export::model::Granularity,

        /// Write output to stdout instead of .code-graph/graph.dot|.mmd.
        #[arg(long)]
        stdout: bool,

        /// Export only files/symbols under this path.
        #[arg(long)]
        root: Option<PathBuf>,

        /// Export a symbol and its N-hop neighborhood.
        #[arg(long)]
        symbol: Option<String>,

        /// Hop depth for --symbol (default: 1).
        #[arg(long, default_value_t = 1)]
        depth: usize,

        /// Exclude paths matching glob patterns (comma-separated).
        #[arg(long, value_delimiter = ',')]
        exclude: Vec<String>,
    },

    /// Show file/directory tree structure with symbol outlines.
    Structure {
        /// Scope output to a specific directory (relative to project root).
        #[arg(long)]
        path: Option<PathBuf>,

        /// Path to the project root (auto-detected from cwd when omitted).
        root: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Maximum directory depth to display (default: 3).
        #[arg(long, default_value_t = 3)]
        depth: usize,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Summarize a single file: role, symbols, imports, dependents.
    #[command(name = "file-summary")]
    FileSummary {
        /// Path to the file to summarize (relative to project root).
        file: PathBuf,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// List all imports of a file, categorized by type.
    Imports {
        /// Path to the file to inspect (relative to project root).
        file: PathBuf,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Detect dead code: unreachable files and unreferenced symbols.
    #[command(name = "dead-code")]
    DeadCode {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Scope analysis to a specific directory (relative to project root).
        #[arg(long)]
        scope: Option<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Detect structural clones: groups of symbols with identical structural signatures.
    ///
    /// Hashes each symbol by (kind, body_size, outgoing edges, incoming edges, decorator count)
    /// and groups symbols with identical hashes.
    Clones {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Minimum number of symbols in a clone group (default: 2).
        #[arg(long, default_value_t = 2)]
        min_group: usize,

        /// Scope analysis to a specific directory (relative to project root).
        #[arg(long)]
        scope: Option<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Compare two graph snapshots and show structural differences.
    Diff {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Name of the base snapshot.
        #[arg(long)]
        from: String,

        /// Name of the target snapshot (defaults to current graph state).
        #[arg(long)]
        to: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Analyze impact of git-changed files on the dependency graph.
    #[command(name = "diff-impact")]
    DiffImpact {
        /// Git ref to diff against (e.g. HEAD~1, main, origin/main).
        base_ref: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Find symbols decorated with a specific decorator/attribute pattern.
    Decorators {
        /// Decorator/attribute name or regex pattern (e.g. "@Component", "derive(Debug)").
        pattern: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Filter by language (rust/rs, typescript/ts, javascript/js, python/py).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,

        /// Filter by framework (e.g. nestjs, angular, fastapi).
        #[arg(long)]
        framework: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Discover functional clusters (groups of related symbols) via graph analysis.
    Clusters {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Scope analysis to a specific directory (relative to project root).
        #[arg(long)]
        scope: Option<PathBuf>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Trace data/call flow paths between two symbols.
    Flow {
        /// Entry (source) symbol name.
        entry: String,

        /// Target (destination) symbol name.
        target: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Maximum number of paths to return (default: 3).
        #[arg(long, default_value_t = 3)]
        max_paths: usize,

        /// Maximum search depth in hops (default: 20).
        #[arg(long, default_value_t = 20)]
        max_depth: usize,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Manage the project registry (add, remove, list, show).
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },

    /// Install Claude Code hooks for transparent code-graph integration.
    ///
    /// Installs PreToolUse hooks into .claude/hooks/ and merges hook configuration
    /// into .claude/settings.json. Cleans up stale MCP configuration automatically.
    Setup {
        /// Install hooks globally (~/.claude/) instead of project-level (.claude/).
        #[arg(long)]
        global: bool,

        /// Remove code-graph hooks and permissions (reverse of setup).
        #[arg(long)]
        uninstall: bool,
    },

    /// Manage the background daemon (start, stop, status).
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Internal: run daemon server (invoked via re-exec, hidden from help).
    #[command(name = "daemon-run", hide = true)]
    DaemonRun {
        /// Path to the project root.
        path: PathBuf,
    },

    /// Plan a symbol rename: list all files and lines that reference the symbol.
    Rename {
        /// Current symbol name to rename.
        symbol: String,

        /// New name for the symbol.
        new_name: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

        /// Use a registered project alias instead of a path.
        #[arg(long)]
        project: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_snapshot_create_parses() {
        let cli = Cli::parse_from(["code-graph", "snapshot", "create", "my-snap"]);
        match cli.command {
            Commands::Snapshot { action } => match action {
                SnapshotAction::Create { name, path } => {
                    assert_eq!(name, "my-snap");
                    assert!(path.is_none(), "path should be None when not specified");
                }
                _ => panic!("expected Create action"),
            },
            _ => panic!("expected Snapshot command"),
        }
    }

    #[test]
    fn test_snapshot_list_parses() {
        let cli = Cli::parse_from(["code-graph", "snapshot", "list"]);
        match cli.command {
            Commands::Snapshot { action } => match action {
                SnapshotAction::List { path } => {
                    assert!(path.is_none(), "path should be None when not specified");
                }
                _ => panic!("expected List action"),
            },
            _ => panic!("expected Snapshot command"),
        }
    }

    #[test]
    fn test_snapshot_delete_parses() {
        let cli = Cli::parse_from(["code-graph", "snapshot", "delete", "my-snap"]);
        match cli.command {
            Commands::Snapshot { action } => match action {
                SnapshotAction::Delete { name, path } => {
                    assert_eq!(name, "my-snap");
                    assert!(path.is_none(), "path should be None when not specified");
                }
                _ => panic!("expected Delete action"),
            },
            _ => panic!("expected Snapshot command"),
        }
    }

    /// Verify that `code-graph index . --no-embeddings` parses correctly when rag feature is on.
    #[test]
    #[cfg(feature = "rag")]
    fn test_index_no_embeddings_flag() {
        let cli = Cli::parse_from(["code-graph", "index", ".", "--no-embeddings"]);
        match cli.command {
            Commands::Index { no_embeddings, .. } => {
                assert!(no_embeddings, "--no-embeddings flag should be true");
            }
            _ => panic!("expected Index command"),
        }
    }

    /// Verify that `code-graph index .` without --no-embeddings defaults to false.
    #[test]
    #[cfg(feature = "rag")]
    fn test_index_embeddings_enabled_by_default() {
        let cli = Cli::parse_from(["code-graph", "index", "."]);
        match cli.command {
            Commands::Index { no_embeddings, .. } => {
                assert!(!no_embeddings, "--no-embeddings should default to false");
            }
            _ => panic!("expected Index command"),
        }
    }

    /// Verify that `code-graph serve` parses with default port and path.
    #[test]
    #[cfg(feature = "web")]
    fn test_serve_parses_defaults() {
        let cli = Cli::parse_from(["code-graph", "serve"]);
        match cli.command {
            Commands::Serve { path, port, .. } => {
                assert_eq!(path, PathBuf::from("."));
                assert_eq!(port, 7070);
            }
            _ => panic!("expected Serve command"),
        }
    }

    /// Verify that `code-graph serve /tmp --port 8080` parses correctly.
    #[test]
    #[cfg(feature = "web")]
    fn test_serve_custom_port_and_path() {
        let cli = Cli::parse_from(["code-graph", "serve", "/tmp", "--port", "8080"]);
        match cli.command {
            Commands::Serve { path, port, .. } => {
                assert_eq!(path, PathBuf::from("/tmp"));
                assert_eq!(port, 8080);
            }
            _ => panic!("expected Serve command"),
        }
    }

    /// Verify that `code-graph serve --ollama` parses when compiled with rag feature.
    #[test]
    #[cfg(all(feature = "web", feature = "rag"))]
    fn test_serve_ollama_flag() {
        let cli = Cli::parse_from(["code-graph", "serve", "--ollama"]);
        match cli.command {
            Commands::Serve { ollama, .. } => {
                assert!(ollama, "--ollama flag should be true");
            }
            _ => panic!("expected Serve command"),
        }
    }

    /// Verify that `code-graph serve` without --ollama defaults to false.
    #[test]
    #[cfg(all(feature = "web", feature = "rag"))]
    fn test_serve_ollama_defaults_to_false() {
        let cli = Cli::parse_from(["code-graph", "serve"]);
        match cli.command {
            Commands::Serve { ollama, .. } => {
                assert!(!ollama, "--ollama should default to false");
            }
            _ => panic!("expected Serve command"),
        }
    }

    #[test]
    fn test_daemon_start_parses() {
        let cli = Cli::parse_from(["code-graph", "daemon", "start"]);
        match cli.command {
            Commands::Daemon { action } => match action {
                DaemonAction::Start { path } => {
                    assert!(path.is_none(), "path should be None when not specified");
                }
                _ => panic!("expected Start action"),
            },
            _ => panic!("expected Daemon command"),
        }
    }

    #[test]
    fn test_daemon_start_with_path_parses() {
        let cli = Cli::parse_from(["code-graph", "daemon", "start", "/tmp/myproject"]);
        match cli.command {
            Commands::Daemon { action } => match action {
                DaemonAction::Start { path } => {
                    assert_eq!(path, Some(PathBuf::from("/tmp/myproject")));
                }
                _ => panic!("expected Start action"),
            },
            _ => panic!("expected Daemon command"),
        }
    }

    #[test]
    fn test_daemon_stop_parses() {
        let cli = Cli::parse_from(["code-graph", "daemon", "stop"]);
        match cli.command {
            Commands::Daemon { action } => match action {
                DaemonAction::Stop { path } => {
                    assert!(path.is_none(), "path should be None when not specified");
                }
                _ => panic!("expected Stop action"),
            },
            _ => panic!("expected Daemon command"),
        }
    }

    #[test]
    fn test_daemon_status_parses() {
        let cli = Cli::parse_from(["code-graph", "daemon", "status"]);
        match cli.command {
            Commands::Daemon { action } => match action {
                DaemonAction::Status { path } => {
                    assert!(path.is_none(), "path should be None when not specified");
                }
                _ => panic!("expected Status action"),
            },
            _ => panic!("expected Daemon command"),
        }
    }

    #[test]
    fn test_daemon_run_parses() {
        let cli = Cli::parse_from(["code-graph", "daemon-run", "/tmp/myproject"]);
        match cli.command {
            Commands::DaemonRun { path } => {
                assert_eq!(path, PathBuf::from("/tmp/myproject"));
            }
            _ => panic!("expected DaemonRun command"),
        }
    }

    // ── Project subcommand tests ─────────────────────────────────────────

    #[test]
    fn test_project_add_parses() {
        let cli = Cli::parse_from(["code-graph", "project", "add", "myproj", "/path/to/proj"]);
        match cli.command {
            Commands::Project { action } => match action {
                ProjectAction::Add { alias, path } => {
                    assert_eq!(alias, "myproj");
                    assert_eq!(path, PathBuf::from("/path/to/proj"));
                }
                _ => panic!("expected Add action"),
            },
            _ => panic!("expected Project command"),
        }
    }

    #[test]
    fn test_project_remove_parses() {
        let cli = Cli::parse_from(["code-graph", "project", "remove", "myproj"]);
        match cli.command {
            Commands::Project { action } => match action {
                ProjectAction::Remove { alias } => {
                    assert_eq!(alias, "myproj");
                }
                _ => panic!("expected Remove action"),
            },
            _ => panic!("expected Project command"),
        }
    }

    #[test]
    fn test_project_list_parses() {
        let cli = Cli::parse_from(["code-graph", "project", "list"]);
        match cli.command {
            Commands::Project { action } => match action {
                ProjectAction::List => {}
                _ => panic!("expected List action"),
            },
            _ => panic!("expected Project command"),
        }
    }

    #[test]
    fn test_project_show_parses() {
        let cli = Cli::parse_from(["code-graph", "project", "show", "myproj"]);
        match cli.command {
            Commands::Project { action } => match action {
                ProjectAction::Show { alias } => {
                    assert_eq!(alias, "myproj");
                }
                _ => panic!("expected Show action"),
            },
            _ => panic!("expected Project command"),
        }
    }

    // ── --project flag on query commands ──────────────────────────────────

    #[test]
    fn test_find_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "find", "MySymbol", "--project", "myproj"]);
        match cli.command {
            Commands::Find {
                symbol,
                project,
                path,
                ..
            } => {
                assert_eq!(symbol, "MySymbol");
                assert_eq!(project, Some("myproj".to_string()));
                assert!(path.is_none());
            }
            _ => panic!("expected Find command"),
        }
    }

    #[test]
    fn test_find_without_project_flag() {
        let cli = Cli::parse_from(["code-graph", "find", "MySymbol"]);
        match cli.command {
            Commands::Find { project, .. } => {
                assert!(project.is_none());
            }
            _ => panic!("expected Find command"),
        }
    }

    #[test]
    fn test_refs_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "refs", "MySymbol", "--project", "myproj"]);
        match cli.command {
            Commands::Refs { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected Refs command"),
        }
    }

    #[test]
    fn test_impact_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "impact", "MySymbol", "--project", "myproj"]);
        match cli.command {
            Commands::Impact { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected Impact command"),
        }
    }

    #[test]
    fn test_stats_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "stats", "--project", "myproj"]);
        match cli.command {
            Commands::Stats { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected Stats command"),
        }
    }

    #[test]
    fn test_context_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "context", "MySymbol", "--project", "myproj"]);
        match cli.command {
            Commands::Context { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected Context command"),
        }
    }

    #[test]
    fn test_circular_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "circular", "--project", "myproj"]);
        match cli.command {
            Commands::Circular { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected Circular command"),
        }
    }

    #[test]
    fn test_dead_code_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "dead-code", "--project", "myproj"]);
        match cli.command {
            Commands::DeadCode { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected DeadCode command"),
        }
    }

    #[test]
    fn test_rename_with_project_flag() {
        let cli = Cli::parse_from(["code-graph", "rename", "old", "new", "--project", "myproj"]);
        match cli.command {
            Commands::Rename { project, .. } => {
                assert_eq!(project, Some("myproj".to_string()));
            }
            _ => panic!("expected Rename command"),
        }
    }
}
