/// Session memory store for the RAG conversational agent.
///
/// Provides per-session message history with LRU (Least Recently Used) eviction
/// when the number of concurrent sessions exceeds the configured capacity.
///
/// # LRU eviction
///
/// Each session tracks `last_used: std::time::Instant`. When creating a new session
/// would exceed `capacity`, the session with the *oldest* `last_used` timestamp is
/// evicted. Accessing a session (via `add_message` or `get_history`) updates its
/// `last_used` so recently-active sessions survive eviction.
///
/// # Thread safety
///
/// `SessionStore` is not `Sync`. The caller (HTTP handler layer in Plan 03) should
/// wrap it in `Arc<tokio::sync::Mutex<SessionStore>>` or `Arc<std::sync::Mutex<SessionStore>>`.
use std::collections::HashMap;
use std::time::Instant;

use uuid::Uuid;

// ─── Message types ─────────────────────────────────────────────────────────────

/// Role of a conversation participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRole {
    #[allow(dead_code)]
    System,
    User,
    Assistant,
}

/// A single message in a conversation.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    #[allow(dead_code)]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content: content.into(),
        }
    }
}

// ─── Session ───────────────────────────────────────────────────────────────────

/// A single conversation session with message history and LRU timestamp.
struct Session {
    messages: Vec<ChatMessage>,
    last_used: Instant,
}

impl Session {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            last_used: Instant::now(),
        }
    }

    fn touch(&mut self) {
        self.last_used = Instant::now();
    }
}

// ─── SessionStore ─────────────────────────────────────────────────────────────

/// LRU-evicting in-memory store for conversation sessions.
///
/// Create with `SessionStore::new(capacity)` where `capacity` is the maximum number
/// of concurrent sessions to retain. Excess sessions are evicted by oldest `last_used`.
pub struct SessionStore {
    sessions: HashMap<String, Session>,
    capacity: usize,
}

impl SessionStore {
    /// Create a new `SessionStore` with the given maximum session `capacity`.
    pub fn new(capacity: usize) -> Self {
        Self {
            sessions: HashMap::new(),
            capacity,
        }
    }

    /// Create a new session and return its UUID `session_id`.
    ///
    /// If the store is at capacity, the least-recently-used session is evicted first.
    pub fn create_session(&mut self) -> String {
        self.evict_if_full();
        let id = Uuid::new_v4().to_string();
        self.sessions.insert(id.clone(), Session::new());
        id
    }

    /// Append `msg` to the session identified by `session_id`.
    ///
    /// Updates `last_used` so this session is considered recently active.
    /// Returns `Err` if `session_id` is not found.
    pub fn add_message(&mut self, session_id: &str, msg: ChatMessage) -> anyhow::Result<()> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("session '{}' not found", session_id))?;
        session.touch();
        session.messages.push(msg);
        Ok(())
    }

    /// Return the message history for `session_id` as a slice.
    ///
    /// Updates `last_used` so this session is considered recently active.
    /// Returns `None` if `session_id` is not found.
    #[allow(dead_code)]
    pub fn get_history(&mut self, session_id: &str) -> Option<&[ChatMessage]> {
        let session = self.sessions.get_mut(session_id)?;
        session.touch();
        Some(&session.messages)
    }

    /// Return the message history without updating last_used (read-only peek).
    pub fn peek_history(&self, session_id: &str) -> Option<&[ChatMessage]> {
        self.sessions.get(session_id).map(|s| s.messages.as_slice())
    }

    /// Returns `true` if a session with the given `session_id` exists.
    pub fn has_session(&self, session_id: &str) -> bool {
        self.sessions.contains_key(session_id)
    }

    /// Returns the number of active sessions.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Returns `true` if there are no active sessions.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    // ── Internal helpers ────────────────────────────────────────────────────────

    /// Evict the least-recently-used session if the store is at or above capacity.
    fn evict_if_full(&mut self) {
        if self.sessions.len() < self.capacity {
            return;
        }
        // Find the session_id with the oldest last_used timestamp.
        let lru_id = self
            .sessions
            .iter()
            .min_by_key(|(_, s)| s.last_used)
            .map(|(id, _)| id.clone());

        if let Some(id) = lru_id {
            self.sessions.remove(&id);
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn session_store_new_creates_empty_store() {
        let store = SessionStore::new(100);
        assert_eq!(store.len(), 0);
        assert!(store.is_empty());
    }

    #[test]
    fn create_session_returns_uuid_string() {
        let mut store = SessionStore::new(10);
        let id = store.create_session();
        // UUID v4 format: 8-4-4-4-12 hex digits
        assert_eq!(id.len(), 36, "UUID should be 36 chars with dashes");
        assert!(store.has_session(&id), "session should be stored");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn add_message_and_get_history_preserves_order() {
        let mut store = SessionStore::new(10);
        let id = store.create_session();

        store
            .add_message(&id, ChatMessage::user("hello"))
            .expect("add user message");
        store
            .add_message(&id, ChatMessage::assistant("hi there"))
            .expect("add assistant message");
        store
            .add_message(&id, ChatMessage::user("how are you?"))
            .expect("add second user message");

        let history = store.get_history(&id).expect("session should exist");
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[0].role, ChatRole::User);
        assert_eq!(history[1].content, "hi there");
        assert_eq!(history[1].role, ChatRole::Assistant);
        assert_eq!(history[2].content, "how are you?");
    }

    #[test]
    fn five_turn_conversation_fully_preserved() {
        let mut store = SessionStore::new(10);
        let id = store.create_session();

        let turns = [
            ("user", "What does foo do?"),
            ("assistant", "foo computes X"),
            ("user", "Where is it defined?"),
            ("assistant", "In src/lib.rs:42"),
            ("user", "Thanks!"),
        ];

        for (role, content) in &turns {
            let msg = if *role == "user" {
                ChatMessage::user(*content)
            } else {
                ChatMessage::assistant(*content)
            };
            store.add_message(&id, msg).expect("add message");
        }

        let history = store.get_history(&id).expect("session exists");
        assert_eq!(history.len(), 5, "all 5 turns must be preserved");
        for (i, (_, expected_content)) in turns.iter().enumerate() {
            assert_eq!(
                history[i].content, *expected_content,
                "turn {} content mismatch",
                i
            );
        }
    }

    #[test]
    fn lru_eviction_removes_oldest_session_when_at_capacity() {
        let mut store = SessionStore::new(3);

        // Create 3 sessions with slight time gaps so last_used timestamps differ.
        let id1 = store.create_session();
        std::thread::sleep(Duration::from_millis(10));
        let id2 = store.create_session();
        std::thread::sleep(Duration::from_millis(10));
        let id3 = store.create_session();
        std::thread::sleep(Duration::from_millis(10));

        assert_eq!(store.len(), 3, "store should be at capacity");

        // Creating a 4th session should evict id1 (oldest, never touched).
        let id4 = store.create_session();

        assert_eq!(
            store.len(),
            3,
            "store should still have 3 sessions after eviction"
        );
        assert!(
            !store.has_session(&id1),
            "id1 (oldest) should have been evicted"
        );
        assert!(store.has_session(&id2), "id2 should still exist");
        assert!(store.has_session(&id3), "id3 should still exist");
        assert!(store.has_session(&id4), "id4 (new) should exist");
    }

    #[test]
    fn accessing_session_updates_last_used_protects_from_eviction() {
        let mut store = SessionStore::new(2);

        // Create 2 sessions.
        let id1 = store.create_session();
        std::thread::sleep(Duration::from_millis(10));
        let id2 = store.create_session();
        std::thread::sleep(Duration::from_millis(10));

        // Touch id1 — it should now have a newer last_used than id2.
        store
            .add_message(&id1, ChatMessage::user("keep me"))
            .expect("add message");
        std::thread::sleep(Duration::from_millis(10));

        // At capacity — creating id3 should evict id2 (oldest last_used).
        let id3 = store.create_session();

        assert_eq!(store.len(), 2);
        assert!(
            store.has_session(&id1),
            "id1 was touched recently — should survive"
        );
        assert!(
            !store.has_session(&id2),
            "id2 was not touched — should be evicted"
        );
        assert!(store.has_session(&id3), "id3 (new) should exist");
    }

    #[test]
    fn add_message_to_unknown_session_returns_error() {
        let mut store = SessionStore::new(10);
        let result = store.add_message("nonexistent-id", ChatMessage::user("hello"));
        assert!(result.is_err(), "adding to unknown session should fail");
    }

    #[test]
    fn get_history_unknown_session_returns_none() {
        let mut store = SessionStore::new(10);
        assert!(store.get_history("no-such-id").is_none());
    }

    #[test]
    fn multiple_sessions_are_independent() {
        let mut store = SessionStore::new(10);
        let id1 = store.create_session();
        let id2 = store.create_session();

        store
            .add_message(&id1, ChatMessage::user("session 1 message"))
            .unwrap();
        store
            .add_message(&id2, ChatMessage::user("session 2 message"))
            .unwrap();

        // Use peek_history (immutable) to avoid double-mutable-borrow.
        let h1: Vec<_> = store.peek_history(&id1).unwrap().to_vec();
        let h2: Vec<_> = store.peek_history(&id2).unwrap().to_vec();

        assert_eq!(h1.len(), 1);
        assert_eq!(h2.len(), 1);
        assert_eq!(h1[0].content, "session 1 message");
        assert_eq!(h2[0].content, "session 2 message");
    }
}
