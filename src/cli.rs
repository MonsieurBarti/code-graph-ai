use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
}
