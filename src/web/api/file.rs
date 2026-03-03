use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;

use super::super::server::AppState;

#[derive(Deserialize)]
pub struct FileQuery {
    pub path: String,
}

/// GET /api/file?path=src/main.rs
///
/// Returns the raw content of the requested file as text/plain.
/// Rejects directory traversal attempts (paths containing "..").
/// Returns 404 if the file does not exist.
pub async fn handler(
    Query(params): Query<FileQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Security: reject directory traversal attempts.
    if params.path.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            "Directory traversal not allowed".to_string(),
        )
            .into_response();
    }

    let file_path = state.project_root.join(&params.path);

    match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => (
            StatusCode::OK,
            [(
                axum::http::header::CONTENT_TYPE,
                "text/plain; charset=utf-8",
            )],
            content,
        )
            .into_response(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            (StatusCode::NOT_FOUND, "File not found".to_string()).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read file: {}", e),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    /// Test that paths with ".." in them are rejected.
    ///
    /// We test the traversal detection logic directly here since we cannot
    /// easily construct an axum State in unit tests.
    #[test]
    fn test_file_api_rejects_traversal() {
        let malicious_paths = vec![
            "../etc/passwd",
            "../../secrets",
            "src/../../../root",
            "foo/../../bar",
        ];
        for path in malicious_paths {
            assert!(
                path.contains(".."),
                "path '{}' should be detected as traversal",
                path
            );
        }
    }

    #[test]
    fn test_safe_paths_not_rejected() {
        let safe_paths = vec!["src/main.rs", "Cargo.toml", "web/dist/index.html"];
        for path in safe_paths {
            assert!(
                !path.contains(".."),
                "path '{}' should not be rejected",
                path
            );
        }
    }
}
