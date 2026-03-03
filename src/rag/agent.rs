/// RAG conversational agent — query classification, LLM orchestration, citation formatting.
///
/// The `RagAgent` is stateless: it orchestrates the full RAG pipeline for a single turn.
/// Session state lives in [`SessionStore`]; the agent reads history and appends new messages
/// after the LLM responds.
///
/// # Query classification
///
/// `classify_query` uses keyword prefix matching to route queries:
/// - **Structural**: "where is", "find", "what calls", "references to", "who uses" — answered
///   using graph tools (symbol lookup, reference tracing).
/// - **Conceptual**: "how", "explain", "what does", "why", "describe" — answered using vector
///   similarity search over pre-computed symbol embeddings.
/// - **Hybrid**: everything else — both graph tools and vector search are used; results are merged.
use genai::Client;
use genai::chat::{ChatMessage as GenAiMessage, ChatRequest};

use crate::graph::CodeGraph;
use crate::rag::embedding::EmbeddingEngine;
use crate::rag::retrieval::{Citation, retrieve, retrieve_structural};
use crate::rag::session::{ChatMessage, ChatRole, SessionStore};
use crate::rag::vector_store::VectorStore;

// ─── QueryKind ────────────────────────────────────────────────────────────────

/// How a user query should be answered — determines which retrieval tools are used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    /// Navigate the code graph (symbol lookup, reference tracing).
    Structural,
    /// Semantic search over symbol embeddings.
    Conceptual,
    /// Use both graph tools and vector search; merge results.
    Hybrid,
}

/// Classify a user query into a `QueryKind` using keyword prefix matching.
///
/// Rules (checked in order):
/// 1. Structural triggers: starts with "where is", "find", "what calls", "references to",
///    "who uses", "show me", "locate"
/// 2. Conceptual triggers: starts with "how", "explain", "what does", "why", "describe",
///    "what is the purpose", "summarize"
/// 3. Hybrid: anything else
pub fn classify_query(query: &str) -> QueryKind {
    let q = query.trim().to_lowercase();

    // Structural: navigation / location queries.
    let structural_prefixes = [
        "where is",
        "find ",
        "what calls",
        "references to",
        "who uses",
        "show me",
        "locate",
    ];
    for prefix in &structural_prefixes {
        if q.starts_with(prefix) {
            return QueryKind::Structural;
        }
    }

    // Conceptual: explanation / understanding queries.
    let conceptual_prefixes = [
        "how ",
        "how does",
        "explain",
        "what does",
        "why ",
        "why is",
        "why does",
        "describe",
        "what is the purpose",
        "summarize",
    ];
    for prefix in &conceptual_prefixes {
        if q.starts_with(prefix) {
            return QueryKind::Conceptual;
        }
    }

    // Default: hybrid retrieval.
    QueryKind::Hybrid
}

// ─── Response types ───────────────────────────────────────────────────────────

/// A fully-formed response from the RAG agent, including the LLM answer and evidence metadata.
#[derive(Debug, Clone)]
pub struct RagResponse {
    /// The LLM-generated answer text.
    pub answer: String,
    /// Evidence items with file/line provenance.
    pub citations: Vec<Citation>,
    /// Names of graph/vector tools invoked during retrieval.
    pub tools_used: Vec<String>,
}

// ─── Prompt builders ──────────────────────────────────────────────────────────

/// Build the system prompt for the RAG agent.
///
/// Instructs the LLM to:
/// - Answer using only the provided codebase context
/// - Use `[N]` footnote citations when referencing evidence
/// - Be concise and focus on the developer's question
pub fn build_system_prompt(project_stats: &str) -> String {
    format!(
        "You are a codebase expert assistant. You answer questions about a specific software project \
         using the provided code context extracted from the project's dependency graph and symbol index.\n\n\
         Project overview:\n{project_stats}\n\n\
         Instructions:\n\
         - Answer only from the provided code context. Do not speculate about code not shown.\n\
         - When referencing a specific symbol, file, or code snippet, add a footnote citation like [1], [2], etc.\n\
         - Keep answers concise and developer-focused.\n\
         - If the context does not contain enough information to answer, say so clearly.\n\
         - Use markdown for code snippets."
    )
}

/// Build the user message for the RAG agent, embedding the retrieved context.
///
/// Combines the retrieved evidence (with `[N]` citation markers) and the user's question.
pub fn build_user_prompt(query: &str, retrieval_context: &str) -> String {
    if retrieval_context.is_empty() {
        query.to_string()
    } else {
        format!("Codebase context:\n{retrieval_context}\n\nQuestion: {query}")
    }
}

// ─── RagAgent ─────────────────────────────────────────────────────────────────

/// Stateless RAG agent that orchestrates the full pipeline for one conversation turn.
///
/// The agent does not own session state — it reads from and writes to a [`SessionStore`]
/// identified by `session_id`. This makes the agent cheap to clone and share.
pub struct RagAgent;

impl RagAgent {
    /// Execute one conversation turn.
    ///
    /// # Steps
    ///
    /// 1. Classify the user query into `Structural`, `Conceptual`, or `Hybrid`.
    /// 2. Retrieve relevant context using the appropriate tools.
    /// 3. Load session history from `session_store`.
    /// 4. Build the message list: `[system, ...history, new_user_message]`.
    /// 5. Call the LLM via `genai::Client::exec_chat`.
    /// 6. Append user message + assistant response to `session_store`.
    /// 7. Return `RagResponse` with answer, citations, and tools_used.
    #[allow(clippy::too_many_arguments)]
    pub async fn ask(
        graph: &CodeGraph,
        vector_store: &VectorStore,
        engine: &EmbeddingEngine,
        session_store: &mut SessionStore,
        session_id: &str,
        query: &str,
        llm_client: &Client,
        model: &str,
    ) -> anyhow::Result<RagResponse> {
        // Step 1: Classify.
        let kind = classify_query(query);

        // Step 2: Retrieve context.
        let retrieval = retrieve(graph, vector_store, engine, query, kind).await?;

        // Step 3: Load session history.
        let history: Vec<ChatMessage> = session_store
            .peek_history(session_id)
            .map(|h| h.to_vec())
            .unwrap_or_default();

        // Step 4: Build messages for the LLM.
        let system_prompt = build_system_prompt("(codebase stats not available)");
        let user_message_content = build_user_prompt(query, &retrieval.context_text);

        let mut messages: Vec<GenAiMessage> = Vec::new();
        // Convert session history to genai messages (skip system messages — they're
        // sent via system field, not as chat turns).
        for msg in &history {
            match msg.role {
                ChatRole::User => messages.push(GenAiMessage::user(&msg.content)),
                ChatRole::Assistant => messages.push(GenAiMessage::assistant(&msg.content)),
                ChatRole::System => {} // System messages are handled separately.
            }
        }
        // Append the new user message with embedded retrieval context.
        messages.push(GenAiMessage::user(&user_message_content));

        let request = ChatRequest::new(messages).with_system(system_prompt);

        // Step 5: Call LLM.
        let response = llm_client.exec_chat(model, request, None).await?;
        let answer = response.first_text().unwrap_or_default().to_string();

        // Step 6: Persist turn to session.
        session_store.add_message(session_id, ChatMessage::user(query))?;
        session_store.add_message(session_id, ChatMessage::assistant(&answer))?;

        // Step 7: Return response.
        Ok(RagResponse {
            answer,
            citations: retrieval.citations,
            tools_used: retrieval.tools_used,
        })
    }

    /// Execute one conversation turn using structural-only retrieval.
    ///
    /// Identical to [`ask`] but skips vector search entirely — no `VectorStore` or
    /// `EmbeddingEngine` required. Used in degraded mode when no vector store is loaded.
    ///
    /// The `tools_used` field in the response includes `"structural-only (no embeddings)"`
    /// to make the retrieval path transparent to callers (RAG-07).
    #[allow(clippy::too_many_arguments)]
    pub async fn ask_structural(
        graph: &CodeGraph,
        session_store: &mut SessionStore,
        session_id: &str,
        query: &str,
        llm_client: &Client,
        model: &str,
    ) -> anyhow::Result<RagResponse> {
        // Structural-only retrieval -- no VectorStore or EmbeddingEngine needed.
        let (context_text, citations, mut tools_used) = retrieve_structural(graph, query);
        // RAG-07 transparency: indicate that vector search was skipped.
        tools_used.push("structural-only (no embeddings)".to_string());

        // Load session history.
        let history: Vec<ChatMessage> = session_store
            .peek_history(session_id)
            .map(|h| h.to_vec())
            .unwrap_or_default();

        // Build LLM messages.
        let system_prompt = build_system_prompt("(codebase stats not available)");
        let user_message_content = build_user_prompt(query, &context_text);

        let mut messages: Vec<GenAiMessage> = Vec::new();
        for msg in &history {
            match msg.role {
                ChatRole::User => messages.push(GenAiMessage::user(&msg.content)),
                ChatRole::Assistant => messages.push(GenAiMessage::assistant(&msg.content)),
                ChatRole::System => {}
            }
        }
        messages.push(GenAiMessage::user(&user_message_content));

        let request = ChatRequest::new(messages).with_system(system_prompt);

        // Call LLM.
        let response = llm_client.exec_chat(model, request, None).await?;
        let answer = response.first_text().unwrap_or_default().to_string();

        // Persist turn to session.
        session_store.add_message(session_id, ChatMessage::user(query))?;
        session_store.add_message(session_id, ChatMessage::assistant(&answer))?;

        Ok(RagResponse {
            answer,
            citations,
            tools_used,
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── classify_query tests ────────────────────────────────────────────────────

    #[test]
    fn classify_query_structural_where_is() {
        assert_eq!(classify_query("where is auth"), QueryKind::Structural);
        assert_eq!(
            classify_query("where is the authentication handler"),
            QueryKind::Structural
        );
    }

    #[test]
    fn classify_query_structural_find() {
        assert_eq!(classify_query("find UserService"), QueryKind::Structural);
        assert_eq!(
            classify_query("Find all instances of fetch_user"),
            QueryKind::Structural
        );
    }

    #[test]
    fn classify_query_structural_what_calls() {
        assert_eq!(
            classify_query("what calls handleAuth"),
            QueryKind::Structural
        );
        assert_eq!(
            classify_query("What calls the login function"),
            QueryKind::Structural
        );
    }

    #[test]
    fn classify_query_structural_references_to() {
        assert_eq!(
            classify_query("references to UserService"),
            QueryKind::Structural
        );
    }

    #[test]
    fn classify_query_structural_who_uses() {
        assert_eq!(
            classify_query("who uses the cache module"),
            QueryKind::Structural
        );
    }

    #[test]
    fn classify_query_conceptual_how_does() {
        assert_eq!(
            classify_query("how does the caching system work"),
            QueryKind::Conceptual
        );
        assert_eq!(
            classify_query("how does authentication work"),
            QueryKind::Conceptual
        );
    }

    #[test]
    fn classify_query_conceptual_explain() {
        assert_eq!(
            classify_query("explain the error handling"),
            QueryKind::Conceptual
        );
        assert_eq!(
            classify_query("Explain the retry mechanism"),
            QueryKind::Conceptual
        );
    }

    #[test]
    fn classify_query_conceptual_why() {
        assert_eq!(
            classify_query("why is this function slow"),
            QueryKind::Conceptual
        );
        assert_eq!(
            classify_query("why does the server crash on startup"),
            QueryKind::Conceptual
        );
    }

    #[test]
    fn classify_query_conceptual_what_does() {
        assert_eq!(
            classify_query("what does the auth module do"),
            QueryKind::Conceptual
        );
    }

    #[test]
    fn classify_query_hybrid_default() {
        // Bare noun phrases default to Hybrid.
        assert_eq!(
            classify_query("database connection pool"),
            QueryKind::Hybrid
        );
        assert_eq!(classify_query("UserService"), QueryKind::Hybrid);
        assert_eq!(classify_query("caching layer"), QueryKind::Hybrid);
    }

    // ── build_system_prompt tests ────────────────────────────────────────────────

    #[test]
    fn build_system_prompt_includes_instructions() {
        let prompt = build_system_prompt("10 files, 42 symbols");
        assert!(
            prompt.contains("codebase"),
            "prompt should mention codebase"
        );
        assert!(
            prompt.contains("[1]") || prompt.contains("[N]"),
            "prompt should mention citation format"
        );
        assert!(
            prompt.contains("10 files, 42 symbols"),
            "prompt should embed project stats"
        );
    }

    // ── build_user_prompt tests ──────────────────────────────────────────────────

    #[test]
    fn build_user_prompt_wraps_context_and_query() {
        let prompt = build_user_prompt("what is foo?", "[1] function foo in src/lib.rs:10");
        assert!(
            prompt.contains("what is foo?"),
            "prompt should contain the query"
        );
        assert!(
            prompt.contains("[1] function foo"),
            "prompt should embed retrieval context"
        );
        assert!(
            prompt.contains("Codebase context:"),
            "prompt should have context header"
        );
    }

    #[test]
    fn build_user_prompt_without_context_returns_query() {
        let prompt = build_user_prompt("what is foo?", "");
        assert_eq!(prompt, "what is foo?");
    }

    // ── Citation serialization test ──────────────────────────────────────────────

    #[test]
    fn citation_has_required_fields() {
        let c = Citation {
            index: 1,
            file_path: "src/auth.rs".to_string(),
            line_start: 42,
            symbol_name: "authenticate_user".to_string(),
        };
        assert_eq!(c.index, 1);
        assert_eq!(c.file_path, "src/auth.rs");
        assert_eq!(c.line_start, 42);
        assert_eq!(c.symbol_name, "authenticate_user");
    }
}
