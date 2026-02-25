use std::path::PathBuf;

/// Output format for graph export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum ExportFormat {
    /// Graphviz DOT format (default). Suitable for large graphs and tooling.
    Dot,
    /// Mermaid flowchart format. Best for small-to-medium graphs in markdown.
    Mermaid,
}

/// Granularity level for exported nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
pub enum Granularity {
    /// One node per symbol (function, struct, etc.). Most detailed; may exceed Mermaid limits.
    Symbol,
    /// One node per file (default). Good balance of detail and readability.
    File,
    /// One node per package/crate. Best for high-level architecture overview.
    Package,
}

impl Default for Granularity {
    fn default() -> Self {
        Granularity::File
    }
}

/// Parameters controlling a graph export operation.
pub struct ExportParams {
    /// Output format: DOT or Mermaid.
    pub format: ExportFormat,
    /// Granularity level: symbol, file, or package.
    pub granularity: Granularity,
    /// Restrict export to nodes whose file paths start with this prefix.
    pub root_filter: Option<PathBuf>,
    /// Export a named symbol and its N-hop neighborhood (BFS outward).
    pub symbol_filter: Option<String>,
    /// Hop depth for --symbol neighborhood BFS (default: 1).
    pub depth: usize,
    /// Exclude files/symbols matching these glob patterns.
    pub exclude_patterns: Vec<String>,
    /// Absolute path to the project root (used for relative path labels and workspace discovery).
    pub project_root: PathBuf,
    /// Write output to stdout instead of a file.
    pub stdout: bool,
}

/// Result of a graph export operation.
pub struct ExportResult {
    /// The rendered graph content (DOT or Mermaid text).
    pub content: String,
    /// Number of nodes in the exported graph (at chosen granularity).
    pub node_count: usize,
    /// Number of edges in the exported graph.
    pub edge_count: usize,
    /// Advisory warnings (e.g. scale guard messages). Already printed to stderr by export_graph.
    pub warnings: Vec<String>,
}
