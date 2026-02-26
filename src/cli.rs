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
        /// Path to the project root (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// List all stored snapshots with creation timestamps.
    List {
        /// Path to the project root (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Delete a named snapshot.
    Delete {
        /// Snapshot name to delete.
        name: String,
        /// Path to the project root (defaults to current directory).
        #[arg(default_value = ".")]
        path: PathBuf,
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
    },

    /// Find a symbol's definition (file:line location).
    ///
    /// Re-indexes the project before executing the query. Supports regex patterns
    /// (e.g. "User.*Service"), case-insensitive matching, kind filters, and file scoping.
    Find {
        /// Symbol name or regex pattern (e.g. "UserService" or "User.*Service").
        symbol: String,

        /// Path to the project root to index and query.
        path: PathBuf,

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

        /// Path to the project root to index and query.
        path: PathBuf,

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

        /// Path to the project root to index and query.
        path: PathBuf,

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
        /// Path to the project root to index and query.
        path: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,

        /// Filter results by language (rust/rs, typescript/ts, javascript/js).
        #[arg(long = "language", alias = "lang")]
        language: Option<String>,
    },

    /// Project statistics overview: file count, symbol breakdown, import summary.
    Stats {
        /// Path to the project root to index and query.
        path: PathBuf,

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

        /// Path to the project root to index and query.
        path: PathBuf,

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

    /// Start an MCP stdio server exposing graph queries as tools for Claude Code.
    Mcp {
        /// Path to the project root (defaults to current directory if omitted).
        path: Option<PathBuf>,
        /// Start a file watcher that auto-reindexes on changes.
        #[arg(long)]
        watch: bool,
    },

    /// Start a file watcher that monitors for changes and re-indexes incrementally.
    ///
    /// Useful for debugging watcher behavior. The MCP server starts its own
    /// embedded watcher automatically — this command runs standalone.
    Watch {
        /// Path to the project root to watch.
        path: PathBuf,
    },

    /// Create, list, or delete graph snapshots for diff comparisons.
    Snapshot {
        #[command(subcommand)]
        action: SnapshotAction,
    },

    /// Export the code graph to DOT or Mermaid format for architectural visualization.
    Export {
        /// Path to the project root to index and export.
        path: PathBuf,

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_mcp_accepts_watch_flag() {
        let cli = Cli::parse_from(["code-graph", "mcp", "--watch"]);
        match cli.command {
            Commands::Mcp { path, watch } => {
                assert!(watch, "--watch flag should be true");
                assert!(path.is_none(), "path should be None when not specified");
            }
            _ => panic!("expected Mcp command"),
        }
    }

    #[test]
    fn test_mcp_without_watch_flag() {
        let cli = Cli::parse_from(["code-graph", "mcp"]);
        match cli.command {
            Commands::Mcp { path, watch } => {
                assert!(!watch, "--watch flag should default to false");
                assert!(path.is_none(), "path should be None when not specified");
            }
            _ => panic!("expected Mcp command"),
        }
    }

    #[test]
    fn test_mcp_with_path_and_watch() {
        let cli = Cli::parse_from(["code-graph", "mcp", "--watch", "/some/path"]);
        match cli.command {
            Commands::Mcp { path, watch } => {
                assert!(watch, "--watch flag should be true");
                assert_eq!(path, Some(PathBuf::from("/some/path")));
            }
            _ => panic!("expected Mcp command"),
        }
    }

    #[test]
    fn test_snapshot_create_parses() {
        let cli = Cli::parse_from(["code-graph", "snapshot", "create", "my-snap"]);
        match cli.command {
            Commands::Snapshot { action } => match action {
                SnapshotAction::Create { name, path } => {
                    assert_eq!(name, "my-snap");
                    assert_eq!(path, PathBuf::from("."));
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
                    assert_eq!(path, PathBuf::from("."));
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
                    assert_eq!(path, PathBuf::from("."));
                }
                _ => panic!("expected Delete action"),
            },
            _ => panic!("expected Snapshot command"),
        }
    }
}
