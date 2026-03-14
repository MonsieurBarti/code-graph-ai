use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use rust_embed::RustEmbed;
use tokio::sync::{RwLock, broadcast};
use tower_http::cors::CorsLayer;

use crate::graph::CodeGraph;

use super::{api, ws};

/// Shared state passed to all axum handlers.
#[derive(Clone)]
pub struct AppState {
    /// The code graph, shared across all handlers (read) and the watcher task (write).
    pub graph: Arc<RwLock<CodeGraph>>,
    /// Absolute path to the project root being served.
    pub project_root: PathBuf,
    /// Broadcast sender for WebSocket push messages (e.g. "graph_updated").
    pub ws_tx: broadcast::Sender<String>,
    /// Single-use auth token required for API access. Generated at startup.
    pub auth_token: String,

    // ── RAG fields (only available when compiled with the `rag` feature) ──────
    //
    // The vector store holds pre-computed symbol embeddings loaded from disk at startup.
    // It is wrapped in Option<> so the server starts gracefully even without an index.
    /// Vector store for symbol embedding search. `None` if no index has been built.
    #[cfg(feature = "rag")]
    pub vector_store: Arc<RwLock<Option<crate::rag::vector_store::VectorStore>>>,
    /// Embedding engine used to embed user queries at chat time.
    /// Wrapped in `Arc<Option<>>` — `None` if engine failed to initialize.
    #[cfg(feature = "rag")]
    pub embedding_engine: Arc<Option<crate::rag::embedding::EmbeddingEngine>>,
    /// Session store for per-session conversation history with LRU eviction.
    #[cfg(feature = "rag")]
    pub session_store: Arc<tokio::sync::Mutex<crate::rag::session::SessionStore>>,
    /// Server-side authentication state (LLM provider + credentials).
    /// Credentials are NEVER sent to the browser.
    #[cfg(feature = "rag")]
    pub auth_state: Arc<RwLock<crate::rag::auth::AuthState>>,
    /// Server-side PKCE state for OAuth flow (verifier + CSRF token).
    /// Not accessible from the browser.
    #[cfg(feature = "rag")]
    pub pkce_state: Arc<tokio::sync::Mutex<crate::web::api::auth::PkceState>>,
}

/// Embedded frontend assets from web/dist/.
#[derive(RustEmbed)]
#[folder = "web/dist/"]
struct WebAssets;

/// Build the axum Router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let router = Router::new()
        .route("/api/graph", get(api::graph::handler))
        .route("/api/file", get(api::file::handler))
        .route("/api/search", get(api::search::handler))
        .route("/api/stats", get(api::stats::handler))
        .route("/ws", get(ws::handler))
        .fallback(serve_asset);

    // Wire RAG routes when compiled with the `rag` feature.
    #[cfg(feature = "rag")]
    let router = router
        .route("/api/chat", axum::routing::post(api::chat::handler))
        .route(
            "/api/auth/status",
            axum::routing::get(api::auth::status_handler),
        )
        .route(
            "/api/auth/key",
            axum::routing::post(api::auth::set_key_handler),
        )
        .route(
            "/api/auth/provider",
            axum::routing::post(api::auth::set_provider_handler),
        )
        .route(
            "/api/auth/oauth/start",
            axum::routing::get(api::auth::oauth_start_handler),
        )
        .route(
            "/api/auth/oauth/callback",
            axum::routing::get(api::auth::oauth_callback_handler),
        )
        .route(
            "/api/ollama/models",
            axum::routing::get(api::auth::ollama_models_handler),
        );

    // Apply security headers middleware.
    let router = router.layer(axum::middleware::from_fn(security_headers));

    // Fix: Restrict CORS to expected local origin instead of wildcard.
    let cors = CorsLayer::new()
        .allow_origin(
            "http://127.0.0.1:3000"
                .parse::<axum::http::HeaderValue>()
                .unwrap(),
        )
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
        ]);

    router.layer(cors).with_state(state)
}

/// Middleware to inject security headers (CSP, X-Content-Type-Options, etc.).
async fn security_headers(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        "Content-Security-Policy",
        "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'"
            .parse()
            .unwrap(),
    );
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("X-Frame-Options", "DENY".parse().unwrap());
    headers.insert("Referrer-Policy", "no-referrer".parse().unwrap());
    response
}

/// Generate a random 32-character hex auth token.
fn generate_auth_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    let a = h.finish();
    let mut h2 = s.build_hasher();
    h2.write_u64(a);
    let b = h2.finish();
    format!("{:016x}{:016x}", a, b)
}

/// Serve embedded frontend assets. Falls back to index.html for unknown paths (SPA routing).
async fn serve_asset(uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Try to serve the exact file first.
    if let Some(content) = WebAssets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.as_ref())
            .body(Body::from(content.data.to_vec()))
            .unwrap_or_else(|_| {
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap()
            })
    } else {
        // SPA fallback: serve index.html for any unknown path.
        if let Some(index) = WebAssets::get("index.html") {
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Body::from(index.data.to_vec()))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap()
                })
        } else {
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not Found"))
                .unwrap()
        }
    }
}

/// Start the axum web server.
///
/// 1. Builds the code graph by indexing `root`.
/// 2. Creates shared AppState with Arc<RwLock<CodeGraph>> + broadcast channel.
/// 3. When compiled with `rag` feature:
///    - Loads vector store from `.code-graph/` if available.
///    - Initializes embedding engine.
///    - Creates session store (capacity 100).
///    - Resolves auth state (Claude or Ollama based on `ollama` flag).
/// 4. Spawns a background watcher task that receives file events, updates the graph,
///    and broadcasts `{"type":"graph_updated"}` to connected WebSocket clients.
/// 5. When compiled with `rag` feature: after graph updates, re-embeds changed file's symbols.
/// 6. Serves on `127.0.0.1:{port}` (localhost only).
///
/// # Parameters
///
/// - `root`   — absolute path to the project root being served.
/// - `port`   — TCP port to listen on.
/// - `ollama` — (rag feature only) if `true`, default LLM provider is Ollama; otherwise Claude.
#[allow(unused_variables)]
pub async fn serve(root: PathBuf, port: u16, ollama: bool) -> anyhow::Result<()> {
    eprintln!("Indexing {}...", root.display());
    let mut graph = crate::build_graph(&root, false)?;
    eprintln!(
        "Indexed {} files, {} symbols.",
        graph.file_count(),
        graph.symbol_count()
    );

    graph.rebuild_bm25_index();

    let (ws_tx, _ws_rx) = broadcast::channel::<String>(64);

    // ── RAG field initialization ───────────────────────────────────────────────
    #[cfg(feature = "rag")]
    let (vector_store, embedding_engine, session_store, auth_state) = {
        // Load vector store from .code-graph/ directory.
        let cache_dir = root.join(".code-graph");
        let vs = match crate::rag::vector_store::VectorStore::load(&cache_dir, 384) {
            Ok(vs) => {
                eprintln!("[rag] Loaded vector index: {} symbols", vs.len());
                Some(vs)
            }
            Err(_) => {
                eprintln!(
                    "[rag] No vector index found. Run 'code-graph index' with --features rag to build embeddings."
                );
                None
            }
        };
        let vector_store = Arc::new(RwLock::new(vs));

        // Initialize embedding engine for query embedding at chat time.
        let engine = match crate::rag::embedding::EmbeddingEngine::try_new() {
            Ok(e) => {
                eprintln!("[rag] Embedding engine initialized.");
                Some(e)
            }
            Err(e) => {
                eprintln!(
                    "[rag] Embedding engine unavailable (queries will use structural retrieval only): {}",
                    e
                );
                None
            }
        };
        let embedding_engine = Arc::new(engine);

        // Session store with 100-session LRU capacity.
        let session_store = Arc::new(tokio::sync::Mutex::new(
            crate::rag::session::SessionStore::new(100),
        ));

        // Resolve auth state.
        let provider = if ollama {
            crate::rag::auth::LlmProvider::Ollama {
                host: "http://localhost:11434".to_string(),
                model: "llama3.2".to_string(),
            }
        } else {
            // Try to resolve Claude API key from env / auth.toml.
            let api_key = crate::rag::auth::resolve_api_key().unwrap_or_default();
            crate::rag::auth::LlmProvider::Claude { api_key }
        };
        let auth_state = Arc::new(RwLock::new(crate::rag::auth::AuthState { provider }));

        (vector_store, embedding_engine, session_store, auth_state)
    };

    // Generate a random single-use auth token for API access.
    let auth_token = generate_auth_token();

    let state = AppState {
        graph: Arc::new(RwLock::new(graph)),
        project_root: root.clone(),
        ws_tx: ws_tx.clone(),
        auth_token: auth_token.clone(),
        #[cfg(feature = "rag")]
        vector_store,
        #[cfg(feature = "rag")]
        embedding_engine,
        #[cfg(feature = "rag")]
        session_store,
        #[cfg(feature = "rag")]
        auth_state,
        #[cfg(feature = "rag")]
        pkce_state: Arc::new(tokio::sync::Mutex::new(
            crate::web::api::auth::PkceState::new(),
        )),
    };

    // ── Background watcher task ────────────────────────────────────────────────
    let watcher_graph = Arc::clone(&state.graph);
    let watcher_root = root.clone();
    let watcher_tx = ws_tx.clone();

    #[cfg(feature = "rag")]
    let watcher_vector_store = Arc::clone(&state.vector_store);
    #[cfg(feature = "rag")]
    let watcher_embedding_engine = Arc::clone(&state.embedding_engine);

    // Start the file watcher, bridging from std channel to tokio channel
    match crate::watcher::start_watcher(&watcher_root) {
        Ok((_handle, std_rx)) => {
            // Bridge: spawn_blocking thread reads from std channel, forwards to tokio channel
            let (bridge_tx, mut bridge_rx) =
                tokio::sync::mpsc::channel::<crate::watcher::event::WatchEvent>(256);
            tokio::task::spawn_blocking(move || {
                while let Ok(event) = std_rx.recv() {
                    if bridge_tx.blocking_send(event).is_err() {
                        return; // receiver dropped
                    }
                }
            });

            // Keep watcher handle alive for the duration of the server
            let _watcher_handle = _handle;

            // Process events from tokio channel (async-safe)
            tokio::spawn(async move {
                while let Some(event) = bridge_rx.recv().await {
                    // Get the file path from the event before the graph write lock takes it.
                    #[cfg(feature = "rag")]
                    let event_file_path: Option<String> = match &event {
                        crate::watcher::event::WatchEvent::Modified(p) => {
                            Some(p.to_string_lossy().to_string())
                        }
                        crate::watcher::event::WatchEvent::Deleted(p) => {
                            Some(p.to_string_lossy().to_string())
                        }
                        _ => None,
                    };

                    {
                        let mut graph = watcher_graph.write().await;
                        crate::watcher::incremental::handle_file_event(
                            &mut graph,
                            &event,
                            &watcher_root,
                        );
                    }

                    // Re-embed changed file's symbols after graph update.
                    #[cfg(feature = "rag")]
                    if let Some(file_path) = event_file_path {
                        let graph = watcher_graph.read().await;
                        let mut vs_guard = watcher_vector_store.write().await;
                        if let (Some(vs), Some(engine)) =
                            (vs_guard.as_mut(), watcher_embedding_engine.as_ref())
                        {
                            match crate::watcher::incremental::re_embed_file(
                                &graph, vs, engine, &file_path,
                            )
                            .await
                            {
                                Ok(count) => {
                                    eprintln!(
                                        "[watch] re-embedded {} symbols from {}",
                                        count, file_path
                                    );
                                }
                                Err(e) => {
                                    eprintln!("[watch] re-embedding failed: {}", e);
                                }
                            }
                        }
                    }

                    let msg = r#"{"type":"graph_updated"}"#.to_string();
                    // Ignore send errors — no clients connected is fine.
                    let _ = watcher_tx.send(msg);
                }
            });
        }
        Err(e) => {
            eprintln!("[watcher] failed to start: {}", e);
        }
    }

    let router = build_router(state);
    // Bind to localhost only — not exposed on all interfaces.
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("Serving on http://127.0.0.1:{port}");
    println!("Auth token: {auth_token}");
    axum::serve(listener, router).await?;
    Ok(())
}
