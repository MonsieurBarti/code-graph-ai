use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;

use crate::daemon::client::query_daemon;
use crate::daemon::pid::{
    is_daemon_running, pid_path, remove_pid_file, remove_socket_file, socket_path,
};
use crate::daemon::protocol::{DaemonRequest, DaemonResponse};
use crate::daemon::server::run_daemon;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal test project with a TypeScript file containing a `greet` function.
fn create_test_project(root: &std::path::Path) {
    std::fs::write(
        root.join("test.ts"),
        "export function greet(name: string): string { return name; }\n",
    )
    .unwrap();
}

/// Wait for the daemon socket to appear, polling every 100ms for up to `timeout`.
async fn wait_for_socket(sock: &std::path::Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if sock.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Send a query to the daemon from a blocking context, avoiding blocking the
/// tokio runtime thread. Returns the deserialized response.
async fn query_blocking(root: &Path, request: DaemonRequest) -> anyhow::Result<DaemonResponse> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || query_daemon(&root, &request))
        .await
        .expect("spawn_blocking panicked")
}

// ---------------------------------------------------------------------------
// 1. Daemon lifecycle test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_daemon_lifecycle() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    // Create test source files.
    create_test_project(&root);

    // Spawn daemon in a background task.
    let daemon_root = root.clone();
    let daemon_handle = tokio::spawn(async move { run_daemon(daemon_root).await });

    // Wait for socket to appear (up to 15s, graph build can be slow in debug mode).
    let sock = socket_path(&root);
    assert!(
        wait_for_socket(&sock, Duration::from_secs(15)).await,
        "daemon socket should appear within timeout"
    );

    // --- Ping ---
    let resp = query_blocking(&root, DaemonRequest::Ping)
        .await
        .expect("Ping should succeed");
    match &resp {
        DaemonResponse::Success { data, .. } => {
            assert_eq!(data["daemon"], "code-graph");
        }
        DaemonResponse::Error { message, .. } => {
            panic!("unexpected error from Ping: {}", message);
        }
    }

    // --- Find for known symbol ---
    let resp = query_blocking(
        &root,
        DaemonRequest::Find {
            symbol: "greet".into(),
            case_insensitive: false,
            kind: vec![],
            file: None,
            language: None,
        },
    )
    .await
    .expect("Find should succeed");
    match &resp {
        DaemonResponse::Success { data, .. } => {
            let arr = data.as_array().expect("find result should be an array");
            assert!(
                !arr.is_empty(),
                "find should return at least one result for 'greet'"
            );
            // Verify the result references the correct symbol name.
            assert_eq!(arr[0]["name"], "greet");
        }
        DaemonResponse::Error { message, .. } => {
            panic!("unexpected error from Find: {}", message);
        }
    }

    // --- Shutdown ---
    let resp = query_blocking(&root, DaemonRequest::Shutdown)
        .await
        .expect("Shutdown should succeed");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "Shutdown should return Success"
    );

    // Wait for daemon to exit.
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;

    // Verify cleanup: PID file and socket should be removed.
    assert!(!sock.exists(), "socket should be removed after shutdown");
    assert!(
        !pid_path(&root).exists(),
        "PID file should be removed after shutdown"
    );
}

// ---------------------------------------------------------------------------
// 2. Fallback test
// ---------------------------------------------------------------------------

#[test]
fn test_fallback_no_daemon() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // No daemon running.
    assert!(
        !is_daemon_running(root),
        "should report not running with no PID file"
    );

    // query_daemon should return Err when no socket exists.
    let result = query_daemon(root, &DaemonRequest::Ping);
    assert!(result.is_err(), "query_daemon should fail with no socket");
}

// ---------------------------------------------------------------------------
// 3. Stale state test
// ---------------------------------------------------------------------------

#[test]
fn test_stale_pid_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Write a stale PID file with a PID that almost certainly does not exist.
    let pid_file = pid_path(root);
    std::fs::create_dir_all(pid_file.parent().unwrap()).unwrap();
    std::fs::write(&pid_file, "99998").unwrap();

    // is_daemon_running should return false for a dead PID.
    assert!(
        !is_daemon_running(root),
        "stale PID (99998) should be detected as not running"
    );

    // Cleanup functions should succeed.
    remove_pid_file(root).unwrap();
    remove_socket_file(root).unwrap();

    // After cleanup, files should not exist.
    assert!(!pid_file.exists(), "PID file should be removed");
    assert!(!socket_path(root).exists(), "socket file should not exist");
}

// ---------------------------------------------------------------------------
// 4. Error handling: malformed JSON
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_malformed_json_returns_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    create_test_project(&root);

    let daemon_root = root.clone();
    let daemon_handle = tokio::spawn(async move { run_daemon(daemon_root).await });

    let sock = socket_path(&root);
    assert!(
        wait_for_socket(&sock, Duration::from_secs(15)).await,
        "daemon socket should appear"
    );

    // Send malformed JSON directly over the socket from a blocking context.
    let sock_path = sock.clone();
    let malformed_resp = tokio::task::spawn_blocking(move || -> DaemonResponse {
        let mut stream = UnixStream::connect(&sock_path).expect("should connect to socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream
            .write_all(b"this is not json\n")
            .expect("should write bad data");
        let mut reader = BufReader::new(&stream);
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .expect("should read response");
        serde_json::from_str(&response).expect("response should be valid JSON")
    })
    .await
    .expect("spawn_blocking panicked");

    assert!(
        matches!(malformed_resp, DaemonResponse::Error { .. }),
        "malformed JSON should return Error response, got: {:?}",
        malformed_resp
    );

    // Verify the daemon is still alive after handling the bad request by sending a Ping.
    let resp = query_blocking(&root, DaemonRequest::Ping)
        .await
        .expect("Ping should still work");
    assert!(
        matches!(resp, DaemonResponse::Success { .. }),
        "daemon should still be alive after malformed request"
    );

    // Clean shutdown.
    let _ = query_blocking(&root, DaemonRequest::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}

// ---------------------------------------------------------------------------
// 5. Input validation: oversized request
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oversized_request_returns_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();

    create_test_project(&root);

    let daemon_root = root.clone();
    let daemon_handle = tokio::spawn(async move { run_daemon(daemon_root).await });

    let sock = socket_path(&root);
    assert!(
        wait_for_socket(&sock, Duration::from_secs(15)).await,
        "daemon socket should appear"
    );

    // Send an oversized request from a blocking context.
    let sock_path = sock.clone();
    let oversize_resp = tokio::task::spawn_blocking(move || -> DaemonResponse {
        let mut stream = UnixStream::connect(&sock_path).expect("should connect to socket");
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(15)))
            .unwrap();

        // Create a payload > 1MB followed by a newline.
        let big_payload = "x".repeat(2_000_000) + "\n";
        stream
            .write_all(big_payload.as_bytes())
            .expect("should write oversized data");

        let mut reader = BufReader::new(&stream);
        let mut response = String::new();
        reader
            .read_line(&mut response)
            .expect("should read error response");
        serde_json::from_str(&response).expect("response should be valid JSON")
    })
    .await
    .expect("spawn_blocking panicked");

    match &oversize_resp {
        DaemonResponse::Error { message, .. } => {
            assert!(
                message.contains("too large") || message.contains("1 MB"),
                "error should mention size limit, got: {}",
                message
            );
        }
        DaemonResponse::Success { .. } => {
            panic!("oversized request should return Error, not Success");
        }
    }

    // Clean shutdown.
    let _ = query_blocking(&root, DaemonRequest::Shutdown).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), daemon_handle).await;
}
