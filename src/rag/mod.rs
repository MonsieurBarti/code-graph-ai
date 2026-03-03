/// RAG (Retrieval-Augmented Generation) module.
///
/// Provides vector indexing via fastembed + usearch, LLM integration via genai,
/// and a conversational agent that grounds answers in the code graph.
///
/// Gated behind the `rag` Cargo feature — compile with `--features rag` to enable.
pub mod agent;
pub mod auth;
pub mod embedding;
pub mod retrieval;
pub mod session;
pub mod vector_store;
