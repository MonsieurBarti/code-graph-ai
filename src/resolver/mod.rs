pub mod file_resolver;
pub mod workspace;

pub use file_resolver::{build_resolver, resolve_import, workspace_map_to_aliases, ResolutionOutcome};
pub use workspace::discover_workspace_packages;
