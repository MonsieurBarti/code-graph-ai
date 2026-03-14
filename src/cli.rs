use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::export;

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

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
    },

    /// Detect dead code: unreachable files and unreferenced symbols.
    #[command(name = "dead-code")]
    DeadCode {
        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

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

    /// Plan a symbol rename: list all files and lines that reference the symbol.
    Rename {
        /// Current symbol name to rename.
        symbol: String,

        /// New name for the symbol.
        new_name: String,

        /// Path to the project root (auto-detected from cwd when omitted).
        path: Option<PathBuf>,

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
}
