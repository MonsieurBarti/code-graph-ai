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
    /// Project root path override
    pub project_path: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct GetStatsParams {
    /// Project root path override
    pub project_path: Option<String>,
}
