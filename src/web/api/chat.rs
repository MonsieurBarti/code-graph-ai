/// POST /api/chat — RAG chat endpoint.
///
/// Accepts a ChatRequest with an optional session_id and user message.
/// Returns a ChatResponse with the LLM-generated answer, citations, and tools used.
///
/// # Session management
///
/// - If `session_id` is provided and exists in the store, the conversation history
///   is preserved across calls.
/// - If `session_id` is absent or unknown, a new session is created and its ID is
///   returned in the response so the client can send it back in future requests.
///
/// # Provider selection
///
/// The request may optionally specify a provider ("claude" or "ollama"). If omitted,
/// the server-side AuthState default is used.
///
/// # Error responses
///
/// - 401 when no Claude API key is configured and the effective provider is Claude.
/// - 500 on internal LLM or retrieval errors.
use std::path::Path;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use genai::Client;
use genai::resolver::{AuthData, AuthResolver};
use serde::{Deserialize, Serialize};

use crate::rag::agent::RagAgent;
use crate::rag::auth::LlmProvider;
use crate::rag::retrieval::Citation;
use crate::web::server::AppState;

// ─── Request / Response DTOs ──────────────────────────────────────────────────

/// Incoming chat request payload.
#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    /// Optional existing session ID to continue a conversation.
    pub session_id: Option<String>,
    /// The user's message / question.
    pub message: String,
}

/// Serializable citation for the HTTP response.
#[derive(Debug, Serialize)]
pub struct CitationDto {
    /// 1-based citation index matching `[N]` markers in `answer`.
    pub index: usize,
    /// Project-relative file path.
    pub file: String,
    /// 1-based line number of the cited symbol.
    pub line: usize,
    /// Symbol identifier name.
    pub symbol: String,
}

/// Chat response payload returned to the client.
#[derive(Debug, Serialize)]
pub struct ChatResponse {
    /// Session ID — echo back in subsequent requests to preserve history.
    pub session_id: String,
    /// LLM-generated answer text (may contain markdown and `[N]` citation markers).
    pub answer: String,
    /// Evidence citations parallel to `[N]` markers in the answer.
    pub citations: Vec<CitationDto>,
    /// Names of retrieval tools invoked during this turn.
    pub tools_used: Vec<String>,
    /// Effective provider used for this response ("claude" or "ollama").
    pub provider: String,
}

// ─── Handler ──────────────────────────────────────────────────────────────────

/// POST /api/chat handler.
pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    // ── 1. Resolve effective provider ──────────────────────────────────────────
    let auth_state = state.auth_state.read().await;
    let effective_provider = auth_state.provider.clone();
    drop(auth_state);

    // ── 2. Validate credentials ────────────────────────────────────────────────
    let (model_name, provider_label): (String, String) = match &effective_provider {
        LlmProvider::Claude { api_key } => {
            if api_key.is_empty() {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "No API key configured. Set ANTHROPIC_API_KEY or use the settings panel."
                    })),
                )
                    .into_response();
            }
            (
                "claude-3-5-sonnet-20241022".to_string(),
                "claude".to_string(),
            )
        }
        LlmProvider::Ollama { model, .. } => (model.clone(), "ollama".to_string()),
    };

    // ── 3. Get or create session ───────────────────────────────────────────────
    let mut session_store = state.session_store.lock().await;
    let session_id = match &req.session_id {
        Some(id) if session_store.has_session(id) => id.clone(),
        _ => session_store.create_session(),
    };
    drop(session_store);

    // ── 4. Get shared state references ────────────────────────────────────────
    let graph = state.graph.read().await;
    let vs_guard = state.vector_store.read().await;

    // Build genai Client with appropriate auth.
    // For Claude: inject the API key via a custom AuthResolver so credentials are never
    // read from ANTHROPIC_API_KEY env var at call time (user may have set a different key
    // via the settings panel).
    let llm_client = match &effective_provider {
        LlmProvider::Claude { api_key } => {
            let key_clone = api_key.clone();
            // AuthResolver::from_resolver_fn accepts fn(ModelIden) -> Result<Option<AuthData>>.
            let auth_resolver = AuthResolver::from_resolver_fn(
                move |_model: genai::ModelIden| -> Result<Option<AuthData>, genai::resolver::Error> {
                    Ok(Some(AuthData::from_single(key_clone.clone())))
                },
            );
            Client::builder().with_auth_resolver(auth_resolver).build()
        }
        LlmProvider::Ollama { .. } => {
            // genai 0.5 connects to Ollama via the default Ollama adapter.
            // The host configuration follows genai's Ollama adapter defaults (localhost:11434).
            Client::default()
        }
    };

    // ── 5. Route to full RAG or degraded structural-only mode ─────────────────
    let engine_ref = state.embedding_engine.as_ref();

    let rag_result = if let (Some(vs), Some(engine)) = (vs_guard.as_ref(), engine_ref) {
        // Full RAG: vector store + embedding engine available.
        let mut session_store = state.session_store.lock().await;
        RagAgent::ask(
            &graph,
            vs,
            engine,
            &mut session_store,
            &session_id,
            &req.message,
            &llm_client,
            &model_name,
        )
        .await
    } else {
        // Degraded mode: no vector store or embedding engine available.
        // Use structural-only retrieval (graph queries only, no vector search).
        drop(vs_guard);
        drop(graph);
        let graph = state.graph.read().await;
        let mut session_store = state.session_store.lock().await;
        RagAgent::ask_structural(
            &graph,
            &mut session_store,
            &session_id,
            &req.message,
            &llm_client,
            &model_name,
        )
        .await
    };

    // ── 7. Map result to HTTP response ────────────────────────────────────────
    match rag_result {
        Ok(rag_response) => {
            // Check for OAuth restriction error from Anthropic.
            let answer = if rag_response
                .answer
                .contains("credential only authorized for Claude Code")
            {
                "Anthropic OAuth is currently restricted. Please use an API key instead."
                    .to_string()
            } else {
                rag_response.answer
            };

            let citations: Vec<CitationDto> = rag_response
                .citations
                .iter()
                .map(|c: &Citation| {
                    // Normalize to project-relative path (graph API uses relative paths).
                    let rel_path = Path::new(&c.file_path)
                        .strip_prefix(&state.project_root)
                        .unwrap_or(Path::new(&c.file_path))
                        .to_string_lossy()
                        .to_string();
                    CitationDto {
                        index: c.index,
                        file: rel_path,
                        line: c.line_start,
                        symbol: c.symbol_name.clone(),
                    }
                })
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!(ChatResponse {
                    session_id,
                    answer,
                    citations,
                    tools_used: rag_response.tools_used,
                    provider: provider_label,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let err_str = e.to_string();
            // Surface OAuth restriction error specifically.
            if err_str.contains("credential only authorized for Claude Code") {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "Anthropic OAuth is currently restricted to Claude.ai and Claude Code CLI. Please use an API key instead."
                    })),
                )
                    .into_response();
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err_str })),
            )
                .into_response()
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_request_deserializes_without_session_id() {
        let json = r#"{"message": "where is auth?"}"#;
        let req: ChatRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.message, "where is auth?");
        assert!(req.session_id.is_none());
    }

    #[test]
    fn chat_request_deserializes_with_session_id() {
        let json = r#"{"session_id": "abc-123", "message": "explain caching"}"#;
        let req: ChatRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.session_id.as_deref(), Some("abc-123"));
        assert_eq!(req.message, "explain caching");
    }

    #[test]
    fn chat_response_serializes_correctly() {
        let resp = ChatResponse {
            session_id: "sess-1".to_string(),
            answer: "The auth module is in src/auth.rs [1].".to_string(),
            citations: vec![CitationDto {
                index: 1,
                file: "src/auth.rs".to_string(),
                line: 42,
                symbol: "authenticate_user".to_string(),
            }],
            tools_used: vec!["find_symbol".to_string()],
            provider: "claude".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        assert!(json.contains("session_id"));
        assert!(json.contains("answer"));
        assert!(json.contains("citations"));
        assert!(json.contains("tools_used"));
        assert!(json.contains("provider"));
    }

    #[test]
    fn citation_dto_has_correct_fields() {
        let c = CitationDto {
            index: 2,
            file: "src/user.rs".to_string(),
            line: 10,
            symbol: "get_user".to_string(),
        };
        assert_eq!(c.index, 2);
        assert_eq!(c.file, "src/user.rs");
        assert_eq!(c.line, 10);
        assert_eq!(c.symbol, "get_user");
    }
}
