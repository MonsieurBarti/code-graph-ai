use crate::rag::auth::LlmProvider;
use crate::web::server::AppState;
/// Auth API handlers — credential management and OAuth PKCE flow.
///
/// Endpoints:
/// - `GET  /api/auth/status`           — returns current provider and whether credentials are configured
/// - `POST /api/auth/key`              — sets the Claude API key server-side (never echoed back)
/// - `POST /api/auth/provider`         — switches between claude and ollama at runtime
/// - `GET  /api/auth/oauth/start`      — initiates PKCE OAuth flow with Anthropic
/// - `GET  /api/auth/oauth/callback`   — exchanges authorization code for token
///
/// # Security
///
/// Credentials are NEVER stored in browser localStorage, cookies, or response bodies.
/// The API key is held exclusively in server-side `AuthState` behind an `Arc<RwLock<>>`.
/// The OAuth PKCE verifier is held in server-side `PkceState` — the browser never sees it.
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect};
use oauth2::{
    AuthorizationCode, ClientId, CsrfToken, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl,
    TokenResponse, basic::BasicClient, reqwest,
};
use serde::{Deserialize, Serialize};

// ─── PKCE state ───────────────────────────────────────────────────────────────

/// Server-side PKCE state stored between /oauth/start and /oauth/callback.
///
/// Not sent to the browser — held exclusively in server memory.
pub struct PkceState {
    /// The PKCE verifier needed to exchange the authorization code for a token.
    pub verifier: Option<PkceCodeVerifier>,
    /// CSRF state token to validate the callback.
    pub csrf_state: Option<String>,
}

impl PkceState {
    pub fn new() -> Self {
        Self {
            verifier: None,
            csrf_state: None,
        }
    }
}

// ─── Request / Response DTOs ──────────────────────────────────────────────────

/// Response body for GET /api/auth/status.
#[derive(Debug, Serialize)]
pub struct AuthStatusResponse {
    /// "claude" or "ollama"
    pub provider: String,
    /// Whether credentials are fully configured (API key set for Claude, host for Ollama).
    pub configured: bool,
    /// Active model name.
    pub model: String,
}

/// Request body for POST /api/auth/key.
#[derive(Debug, Deserialize)]
pub struct SetKeyRequest {
    /// The new Anthropic API key (e.g. "sk-ant-...").
    pub api_key: String,
}

/// Request body for POST /api/auth/provider.
#[derive(Debug, Deserialize)]
pub struct SetProviderRequest {
    /// "claude" or "ollama"
    pub provider: String,
    /// Optional model override (e.g. "llama3.2" for Ollama).
    pub model: Option<String>,
}

/// Query parameters for GET /api/auth/oauth/callback.
#[derive(Debug, Deserialize)]
pub struct OAuthCallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/auth/status — returns current provider configuration.
pub async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let auth = state.auth_state.read().await;
    let (provider, configured, model) = match &auth.provider {
        LlmProvider::Claude { api_key } => (
            "claude".to_string(),
            !api_key.is_empty(),
            "claude-3-5-sonnet-20241022".to_string(),
        ),
        LlmProvider::Ollama { host, model } => {
            ("ollama".to_string(), !host.is_empty(), model.clone())
        }
    };

    (
        StatusCode::OK,
        Json(AuthStatusResponse {
            provider,
            configured,
            model,
        }),
    )
}

/// POST /api/auth/key — store API key server-side. Never echoes key back.
pub async fn set_key_handler(
    State(state): State<AppState>,
    Json(req): Json<SetKeyRequest>,
) -> impl IntoResponse {
    if req.api_key.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "api_key must not be empty"})),
        )
            .into_response();
    }

    let mut auth = state.auth_state.write().await;
    auth.provider = LlmProvider::Claude {
        api_key: req.api_key,
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "ok", "provider": "claude"})),
    )
        .into_response()
}

/// POST /api/auth/provider — switch between claude and ollama at runtime.
pub async fn set_provider_handler(
    State(state): State<AppState>,
    Json(req): Json<SetProviderRequest>,
) -> impl IntoResponse {
    let mut auth = state.auth_state.write().await;
    match req.provider.as_str() {
        "claude" => {
            // Preserve existing API key if we're switching back to Claude.
            let existing_key = match &auth.provider {
                LlmProvider::Claude { api_key } => api_key.clone(),
                _ => String::new(),
            };
            auth.provider = LlmProvider::Claude {
                api_key: existing_key,
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "provider": "claude"})),
            )
                .into_response()
        }
        "ollama" => {
            let model = req.model.unwrap_or_else(|| "llama3.2".to_string());
            auth.provider = LlmProvider::Ollama {
                host: "http://localhost:11434".to_string(),
                model,
            };
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "provider": "ollama"})),
            )
                .into_response()
        }
        other => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unknown provider '{}'; expected 'claude' or 'ollama'", other)
            })),
        )
            .into_response(),
    }
}

/// GET /api/auth/oauth/start — initiate PKCE OAuth flow with Anthropic.
///
/// Generates a PKCE challenge, stores the verifier server-side, and redirects
/// the browser to the Anthropic authorization URL.
///
/// NOTE: Anthropic's OAuth is currently restricted to Claude.ai and Claude Code CLI.
/// This endpoint is implemented for completeness. The callback will surface the
/// restriction error if the token is rejected.
pub async fn oauth_start_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Anthropic OAuth endpoints (well-known values).
    const ANTHROPIC_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
    const ANTHROPIC_TOKEN_URL: &str = "https://claude.ai/oauth/token";
    // Redirect URI must match what's configured in the Anthropic OAuth application.
    const REDIRECT_URI: &str = "http://localhost:7070/api/auth/oauth/callback";

    let client = match build_oauth_client(ANTHROPIC_AUTH_URL, ANTHROPIC_TOKEN_URL, REDIRECT_URI) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("OAuth client error: {}", e)})),
            )
                .into_response();
        }
    };

    // Generate PKCE challenge.
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    // Generate authorization URL + CSRF token.
    let (auth_url, csrf_token) = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge)
        .url();

    // Store PKCE verifier server-side.
    {
        let mut pkce = state.pkce_state.lock().await;
        pkce.verifier = Some(pkce_verifier);
        pkce.csrf_state = Some(csrf_token.secret().clone());
    }

    // Redirect browser to Anthropic's authorization URL.
    Redirect::temporary(auth_url.as_str()).into_response()
}

/// GET /api/auth/oauth/callback — exchange authorization code for token.
///
/// On success: stores the token in AuthState.
/// On Anthropic rejection (OAuth restriction): returns an error page with instructions
/// to use an API key instead.
pub async fn oauth_callback_handler(
    State(state): State<AppState>,
    Query(params): Query<OAuthCallbackParams>,
) -> impl IntoResponse {
    // Handle authorization server errors (e.g. user denied).
    if let Some(error) = &params.error {
        return error_page(
            &format!(
                "Authorization failed: {}. Please use an API key instead.",
                error
            ),
            Some("/"),
        )
        .into_response();
    }

    let code = match &params.code {
        Some(c) => c.clone(),
        None => {
            return error_page("No authorization code received.", Some("/")).into_response();
        }
    };

    // Validate CSRF state.
    let pkce_state = state.pkce_state.lock().await;
    if let Some(expected_state) = &pkce_state.csrf_state {
        let received_state = params.state.as_deref().unwrap_or("");
        if expected_state != received_state {
            return error_page("Invalid OAuth state (CSRF mismatch).", Some("/")).into_response();
        }
    }

    let verifier = match pkce_state.verifier.as_ref() {
        Some(v) => PkceCodeVerifier::new(v.secret().clone()),
        None => {
            return error_page("OAuth session expired. Please try again.", Some("/"))
                .into_response();
        }
    };
    drop(pkce_state);

    // Exchange authorization code for token.
    const ANTHROPIC_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
    const ANTHROPIC_TOKEN_URL: &str = "https://claude.ai/oauth/token";
    const REDIRECT_URI: &str = "http://localhost:7070/api/auth/oauth/callback";

    let client = match build_oauth_client(ANTHROPIC_AUTH_URL, ANTHROPIC_TOKEN_URL, REDIRECT_URI) {
        Ok(c) => c,
        Err(e) => {
            return error_page(
                &format!("OAuth client error: {}. Please use an API key instead.", e),
                Some("/"),
            )
            .into_response();
        }
    };

    let http_client = reqwest::async_http_client;

    let token_result = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(verifier)
        .request_async(http_client)
        .await;

    match token_result {
        Ok(token) => {
            let access_token = token.access_token().secret().clone();
            let mut auth = state.auth_state.write().await;
            auth.provider = LlmProvider::Claude {
                api_key: access_token,
            };
            // Redirect back to chat UI.
            Redirect::temporary("/").into_response()
        }
        Err(e) => {
            let err_str = e.to_string();
            // Detect Anthropic's OAuth restriction error.
            let msg = if err_str.contains("unauthorized")
                || err_str.contains("restricted")
                || err_str.contains("Claude Code")
            {
                "Anthropic OAuth is currently restricted to Claude.ai and Claude Code CLI. \
                 Please use an API key instead."
                    .to_string()
            } else {
                format!(
                    "Token exchange failed: {}. Please use an API key instead.",
                    err_str
                )
            };
            error_page(&msg, Some("/")).into_response()
        }
    }
}

/// GET /api/ollama/models — proxy to Ollama's /api/tags to list locally available models.
///
/// The browser can't call localhost:11434 directly due to CORS, so we proxy it server-side.
pub async fn ollama_models_handler(State(state): State<AppState>) -> impl IntoResponse {
    // Determine Ollama host from current auth state (or default).
    let host = {
        let auth = state.auth_state.read().await;
        match &auth.provider {
            LlmProvider::Ollama { host, .. } => host.clone(),
            _ => "http://localhost:11434".to_string(),
        }
    };

    let url = format!("{}/api/tags", host.trim_end_matches('/'));

    // Use a simple TCP-based HTTP GET — avoids adding reqwest as a direct dependency.
    match tokio_ollama_get(&url).await {
        Ok(body) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": format!("Cannot reach Ollama at {}: {}", host, e),
                "models": []
            })),
        )
            .into_response(),
    }
}

/// Minimal async HTTP GET using tokio TCP — avoids adding reqwest as a direct dep.
async fn tokio_ollama_get(url: &str) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    // Parse URL (only supports http://host:port/path).
    let url = url
        .strip_prefix("http://")
        .ok_or("Only http:// URLs supported")?;
    let (host_port, path) = url.split_once('/').unwrap_or((url, "api/tags"));
    let path = format!("/{}", path);

    let mut stream = TcpStream::connect(host_port)
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    let host = host_port.split(':').next().unwrap_or(host_port);
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| format!("Write failed: {}", e))?;

    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .await
        .map_err(|e| format!("Read failed: {}", e))?;

    let response = String::from_utf8_lossy(&buf);
    // Extract body after \r\n\r\n header separator.
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();

    // Handle chunked transfer encoding — strip chunk size lines.
    if response.contains("Transfer-Encoding: chunked") {
        let mut decoded = String::new();
        let mut remaining = body.as_str();
        loop {
            let (size_str, rest) = remaining.split_once("\r\n").unwrap_or(("0", ""));
            let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            if rest.len() >= size {
                decoded.push_str(&rest[..size]);
                remaining = &rest[size..];
                // Skip trailing \r\n after chunk data.
                remaining = remaining.strip_prefix("\r\n").unwrap_or(remaining);
            } else {
                break;
            }
        }
        Ok(decoded)
    } else {
        Ok(body)
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Build an OAuth2 BasicClient for the Anthropic provider.
fn build_oauth_client(
    auth_url: &str,
    token_url: &str,
    redirect_uri: &str,
) -> anyhow::Result<BasicClient> {
    let client_id = ClientId::new("code-graph-rag".to_string());
    let auth_endpoint = oauth2::AuthUrl::new(auth_url.to_string())
        .map_err(|e| anyhow::anyhow!("invalid auth URL: {}", e))?;
    let token_endpoint = oauth2::TokenUrl::new(token_url.to_string())
        .map_err(|e| anyhow::anyhow!("invalid token URL: {}", e))?;
    let redirect = RedirectUrl::new(redirect_uri.to_string())
        .map_err(|e| anyhow::anyhow!("invalid redirect URI: {}", e))?;

    Ok(
        BasicClient::new(client_id, None, auth_endpoint, Some(token_endpoint))
            .set_redirect_uri(redirect),
    )
}

/// Return an HTML error page with a link back to the chat panel.
fn error_page(message: &str, back_link: Option<&str>) -> impl IntoResponse {
    let back_href = back_link.unwrap_or("/");
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head><title>Authentication Error</title></head>
<body>
  <h2>Authentication Error</h2>
  <p>{message}</p>
  <p><a href="{back_href}">Return to chat</a></p>
</body>
</html>"#
    );
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

// ─── AppState extension: PkceState ────────────────────────────────────────────
//
// PkceState is added to AppState in server.rs. It is declared here for cohesion
// because it is only used by auth handlers.

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_status_response_serializes() {
        let resp = AuthStatusResponse {
            provider: "claude".to_string(),
            configured: true,
            model: "claude-3-5-sonnet-20241022".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        assert!(json.contains("provider"));
        assert!(json.contains("configured"));
        assert!(json.contains("model"));
    }

    #[test]
    fn set_key_request_deserializes() {
        let json = r#"{"api_key": "sk-ant-test-key"}"#;
        let req: SetKeyRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.api_key, "sk-ant-test-key");
    }

    #[test]
    fn set_provider_request_deserializes_ollama() {
        let json = r#"{"provider": "ollama", "model": "mistral"}"#;
        let req: SetProviderRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.provider, "ollama");
        assert_eq!(req.model.as_deref(), Some("mistral"));
    }

    #[test]
    fn set_provider_request_deserializes_claude_no_model() {
        let json = r#"{"provider": "claude"}"#;
        let req: SetProviderRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.provider, "claude");
        assert!(req.model.is_none());
    }

    #[test]
    fn pkce_state_new_is_empty() {
        let s = PkceState::new();
        assert!(s.verifier.is_none());
        assert!(s.csrf_state.is_none());
    }
}
