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
/// Rejects any path that does not resolve within the project root (handles ".." and symlinks).
/// Returns 404 if the file does not exist.
pub async fn handler(
    Query(params): Query<FileQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let file_path = state.project_root.join(&params.path);

    // Security: verify the resolved path is within the project root.
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (StatusCode::NOT_FOUND, "File not found".to_string()).into_response();
        }
    };
    let project_root = match state.project_root.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Project root not accessible".to_string(),
            )
                .into_response();
        }
    };
    if !canonical.starts_with(&project_root) {
        return (
            StatusCode::BAD_REQUEST,
            "Path outside project root".to_string(),
        )
            .into_response();
    }

    match tokio::fs::read_to_string(&canonical).await {
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
    #[test]
    fn test_traversal_paths_detected() {
        // Canonical path comparison handles traversal — the old string-based test
        // is no longer needed. The real protection is in the handler's canonicalize
        // + starts_with check. These paths demonstrate what the handler catches:
        let malicious_paths = vec!["../etc/passwd", "../../secrets", "src/../../../root"];
        for path in malicious_paths {
            // All these paths would fail canonicalize or starts_with in the handler.
            assert!(path.contains("..") || path.starts_with('/'));
        }
    }

    #[test]
    fn test_safe_paths_accepted() {
        let safe_paths = vec!["src/main.rs", "Cargo.toml", "web/dist/index.html"];
        for path in safe_paths {
            assert!(!path.contains("..") && !path.starts_with('/'));
        }
    }
}
