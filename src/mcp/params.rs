use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
pub struct FindSymbolParams {
    /// Symbol name or regex pattern
    pub symbol: String,
    /// File/directory scope (relative to project root)
    pub path: Option<String>,
    /// Filter by kind: function, class, interface, type, enum, variable, component
    pub kind: Option<String>,
    /// Max results (default: 20)
    pub limit: Option<usize>,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindReferencesParams {
    /// Symbol name or regex pattern
    pub symbol: String,
    /// Max results (default: 30)
    pub limit: Option<usize>,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetImpactParams {
    /// Symbol name or regex pattern
    pub symbol: String,
    /// Max affected files (default: 50)
    pub limit: Option<usize>,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct DetectCircularParams {
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetContextParams {
    /// Symbol name or regex pattern
    pub symbol: String,
    /// Sections to include: r=references, c=callers, e=callees, x=extends, i=implements, X=extended-by, I=implemented-by.
    /// Definitions always included. Omit for all sections.
    pub sections: Option<String>,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetStatsParams {
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetStructureParams {
    /// Directory or file path to show structure for (relative to project root, or absolute).
    /// Omit for entire project.
    pub path: Option<String>,
    /// Tree depth limit — number of directory levels below the starting path (default: 3).
    /// No hard cap — truncation handles overflow.
    pub depth: Option<usize>,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetFileSummaryParams {
    /// Path to the file (relative to project root, or absolute)
    pub path: String,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetImportsParams {
    /// Path to the file (relative to project root, or absolute)
    pub path: String,
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportGraphParams {
    /// Output format: "dot" (default) or "mermaid"
    pub format: Option<String>,
    /// Granularity: "file" (default), "symbol", or "package"
    pub granularity: Option<String>,
    /// Filter to files/symbols under this path
    pub root: Option<String>,
    /// Export a specific symbol and its neighborhood
    pub symbol: Option<String>,
    /// Hop depth for symbol neighborhood (default: 1)
    pub depth: Option<usize>,
    /// Exclude paths matching glob patterns (comma-separated)
    pub exclude: Option<String>,
    /// Project root path override
    pub project_path: Option<String>,
}
