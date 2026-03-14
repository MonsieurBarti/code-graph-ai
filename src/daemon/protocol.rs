use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Protocol version for forward compatibility.
pub const PROTOCOL_VERSION: u32 = 1;

/// A request from the CLI client to the daemon.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum DaemonRequest {
    /// Health check — daemon responds with version info.
    Ping,
    /// Graceful shutdown — daemon cleans up and exits.
    Shutdown,

    // -- Query commands (mirror CLI subcommands that call load_or_build) --
    Find {
        symbol: String,
        #[serde(default)]
        case_insensitive: bool,
        #[serde(default)]
        kind: Vec<String>,
        file: Option<PathBuf>,
        language: Option<String>,
    },
    Refs {
        symbol: String,
        #[serde(default)]
        case_insensitive: bool,
        #[serde(default)]
        kind: Vec<String>,
        file: Option<PathBuf>,
        language: Option<String>,
    },
    Impact {
        symbol: String,
        #[serde(default)]
        case_insensitive: bool,
        #[serde(default)]
        tree: bool,
        language: Option<String>,
    },
    Context {
        symbol: String,
        #[serde(default)]
        case_insensitive: bool,
        language: Option<String>,
    },
    Stats {
        language: Option<String>,
    },
    Circular {
        language: Option<String>,
    },
    DeadCode {
        scope: Option<PathBuf>,
    },
    Clones {
        scope: Option<PathBuf>,
        #[serde(default = "default_min_group")]
        min_group: usize,
    },
    Export {
        format: String,
        granularity: String,
        #[serde(default)]
        stdout: bool,
        root: Option<PathBuf>,
        symbol: Option<String>,
        #[serde(default = "default_depth")]
        depth: usize,
        #[serde(default)]
        exclude: Vec<String>,
    },
    Structure {
        path: Option<PathBuf>,
        #[serde(default = "default_structure_depth")]
        depth: usize,
    },
    FileSummary {
        file: PathBuf,
    },
    Imports {
        file: PathBuf,
    },
    Diff {
        from: String,
        to: Option<String>,
    },
    DiffImpact {
        base_ref: String,
    },
    Decorators {
        pattern: String,
        language: Option<String>,
        framework: Option<String>,
    },
    Clusters {
        scope: Option<PathBuf>,
    },
    Flow {
        entry: String,
        target: String,
        #[serde(default = "default_max_paths")]
        max_paths: usize,
        #[serde(default = "default_max_depth")]
        max_depth: usize,
    },
    Rename {
        symbol: String,
        new_name: String,
    },
    SnapshotCreate {
        name: String,
    },
    SnapshotList,
    SnapshotDelete {
        name: String,
    },
}

fn default_min_group() -> usize {
    2
}
fn default_depth() -> usize {
    1
}
fn default_structure_depth() -> usize {
    3
}
fn default_max_paths() -> usize {
    3
}
fn default_max_depth() -> usize {
    20
}

/// A response from the daemon to the CLI client.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DaemonResponse {
    /// Successful query result.
    Success {
        /// Protocol version of the daemon.
        version: u32,
        /// Query result as opaque JSON value.
        data: serde_json::Value,
    },
    /// Error response.
    Error {
        /// Protocol version of the daemon.
        version: u32,
        /// Human-readable error message.
        message: String,
    },
}

impl DaemonResponse {
    /// Create a success response with the given data.
    pub fn success(data: serde_json::Value) -> Self {
        Self::Success {
            version: PROTOCOL_VERSION,
            data,
        }
    }

    /// Create an error response with the given message.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            version: PROTOCOL_VERSION,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ping_roundtrip() {
        let req = DaemonRequest::Ping;
        let json = serde_json::to_string(&req).unwrap();
        let parsed: DaemonRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DaemonRequest::Ping));
    }

    #[test]
    fn request_shutdown_roundtrip() {
        let req = DaemonRequest::Shutdown;
        let json = serde_json::to_string(&req).unwrap();
        let parsed: DaemonRequest = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, DaemonRequest::Shutdown));
    }

    #[test]
    fn request_find_roundtrip() {
        let req = DaemonRequest::Find {
            symbol: "UserService".into(),
            case_insensitive: true,
            kind: vec!["function".into()],
            file: Some(PathBuf::from("src/main.rs")),
            language: Some("rust".into()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: DaemonRequest = serde_json::from_str(&json).unwrap();
        match parsed {
            DaemonRequest::Find {
                symbol,
                case_insensitive,
                kind,
                file,
                language,
            } => {
                assert_eq!(symbol, "UserService");
                assert!(case_insensitive);
                assert_eq!(kind, vec!["function"]);
                assert_eq!(file, Some(PathBuf::from("src/main.rs")));
                assert_eq!(language, Some("rust".into()));
            }
            _ => panic!("expected Find"),
        }
    }

    #[test]
    fn response_success_roundtrip() {
        let resp = DaemonResponse::success(serde_json::json!({"symbols": []}));
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: DaemonResponse = serde_json::from_str(&json).unwrap();
        match parsed {
            DaemonResponse::Success { version, data } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(data, serde_json::json!({"symbols": []}));
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn response_error_roundtrip() {
        let resp = DaemonResponse::error("something broke");
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: DaemonResponse = serde_json::from_str(&json).unwrap();
        match parsed {
            DaemonResponse::Error { version, message } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(message, "something broke");
            }
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn all_query_variants_serialize() {
        // Ensure every variant can be serialized without panic.
        let variants: Vec<DaemonRequest> = vec![
            DaemonRequest::Ping,
            DaemonRequest::Shutdown,
            DaemonRequest::Find {
                symbol: "X".into(),
                case_insensitive: false,
                kind: vec![],
                file: None,
                language: None,
            },
            DaemonRequest::Refs {
                symbol: "X".into(),
                case_insensitive: false,
                kind: vec![],
                file: None,
                language: None,
            },
            DaemonRequest::Impact {
                symbol: "X".into(),
                case_insensitive: false,
                tree: false,
                language: None,
            },
            DaemonRequest::Context {
                symbol: "X".into(),
                case_insensitive: false,
                language: None,
            },
            DaemonRequest::Stats { language: None },
            DaemonRequest::Circular { language: None },
            DaemonRequest::DeadCode { scope: None },
            DaemonRequest::Clones {
                scope: None,
                min_group: 2,
            },
            DaemonRequest::Export {
                format: "dot".into(),
                granularity: "file".into(),
                stdout: false,
                root: None,
                symbol: None,
                depth: 1,
                exclude: vec![],
            },
            DaemonRequest::Structure {
                path: None,
                depth: 3,
            },
            DaemonRequest::FileSummary {
                file: PathBuf::from("src/main.rs"),
            },
            DaemonRequest::Imports {
                file: PathBuf::from("src/main.rs"),
            },
            DaemonRequest::Diff {
                from: "snap1".into(),
                to: None,
            },
            DaemonRequest::DiffImpact {
                base_ref: "HEAD~1".into(),
            },
            DaemonRequest::Decorators {
                pattern: "@Component".into(),
                language: None,
                framework: None,
            },
            DaemonRequest::Clusters { scope: None },
            DaemonRequest::Flow {
                entry: "A".into(),
                target: "B".into(),
                max_paths: 3,
                max_depth: 20,
            },
            DaemonRequest::Rename {
                symbol: "old".into(),
                new_name: "new".into(),
            },
            DaemonRequest::SnapshotCreate {
                name: "snap".into(),
            },
            DaemonRequest::SnapshotList,
            DaemonRequest::SnapshotDelete {
                name: "snap".into(),
            },
        ];

        for variant in &variants {
            let json = serde_json::to_string(variant).unwrap();
            let _parsed: DaemonRequest = serde_json::from_str(&json).unwrap();
        }
        // 23 variants total (Ping + Shutdown + 21 query types)
        assert_eq!(variants.len(), 23);
    }
}
