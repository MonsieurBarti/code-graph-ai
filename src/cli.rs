use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

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
    /// Index a project directory, discovering and parsing all TypeScript/JavaScript files.
    Index {
        /// Path to the project root to index.
        path: PathBuf,

        /// Print each discovered file path during indexing.
        #[arg(short, long)]
        verbose: bool,

        /// Output results as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
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
    },

    /// Project statistics overview: file count, symbol breakdown, import summary.
    Stats {
        /// Path to the project root to index and query.
        path: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Compact)]
        format: OutputFormat,
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
    },
}
