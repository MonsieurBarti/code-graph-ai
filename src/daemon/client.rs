use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use super::pid::socket_path;
use super::protocol::{DaemonRequest, DaemonResponse};

/// Query the daemon over its Unix socket.
///
/// Connects to `.code-graph/daemon.sock`, writes the request as a JSON line,
/// reads a JSON line response, and deserializes it. Returns `Err` (not panic)
/// when the socket does not exist or the connection is refused.
pub fn query_daemon(project_root: &Path, request: &DaemonRequest) -> Result<DaemonResponse> {
    let sock = socket_path(project_root);

    let stream = UnixStream::connect(&sock)
        .with_context(|| format!("failed to connect to daemon socket {}", sock.display()))?;

    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .context("failed to set write timeout")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .context("failed to set read timeout")?;

    // Write request as a single JSON line.
    let mut writer = stream.try_clone().context("failed to clone stream")?;
    let mut line = serde_json::to_string(request).context("failed to serialize request")?;
    line.push('\n');
    writer
        .write_all(line.as_bytes())
        .context("failed to write request to daemon")?;
    writer.flush().context("failed to flush request")?;

    // Read response as a single JSON line.
    let reader = BufReader::new(stream);
    let mut response_line = String::new();
    let mut reader = reader;
    reader
        .read_line(&mut response_line)
        .context("failed to read response from daemon")?;

    if response_line.is_empty() {
        anyhow::bail!("daemon closed connection without responding");
    }

    let response: DaemonResponse =
        serde_json::from_str(&response_line).context("failed to deserialize daemon response")?;

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn query_daemon_returns_err_when_socket_missing() {
        let tmp = TempDir::new().unwrap();
        let result = query_daemon(tmp.path(), &DaemonRequest::Ping);
        assert!(result.is_err(), "expected Err when socket does not exist");
    }

    #[test]
    fn query_daemon_does_not_panic_on_missing_socket() {
        // Verify graceful error, not panic.
        let tmp = TempDir::new().unwrap();
        let _ = query_daemon(tmp.path(), &DaemonRequest::Shutdown);
    }
}
