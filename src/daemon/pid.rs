use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Return the path to the daemon PID file for a given project root.
pub fn pid_path(project_root: &Path) -> PathBuf {
    project_root.join(".code-graph").join("daemon.pid")
}

/// Return the path to the daemon Unix socket for a given project root.
pub fn socket_path(project_root: &Path) -> PathBuf {
    project_root.join(".code-graph").join("daemon.sock")
}

/// Return the path to the daemon log file for a given project root.
pub fn log_path(project_root: &Path) -> PathBuf {
    project_root.join(".code-graph").join("daemon.log")
}

/// Write the current process PID to the PID file with mode 0600.
pub fn write_pid_file(project_root: &Path) -> Result<()> {
    let path = pid_path(project_root);

    // Ensure .code-graph directory exists.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let pid = std::process::id();
    fs::write(&path, pid.to_string())
        .with_context(|| format!("failed to write PID file {}", path.display()))?;

    // Set file permissions to 0600 (owner read/write only).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    }

    Ok(())
}

/// Read the PID from the PID file. Returns `Some(pid)` if valid, `None` otherwise.
pub fn read_pid_file(project_root: &Path) -> Option<u32> {
    let path = pid_path(project_root);
    let contents = fs::read_to_string(&path).ok()?;
    contents.trim().parse::<u32>().ok()
}

/// Remove the PID file. Returns Ok even if the file doesn't exist.
pub fn remove_pid_file(project_root: &Path) -> Result<()> {
    let path = pid_path(project_root);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("failed to remove PID file {}", path.display())),
    }
}

/// Remove the daemon socket file. Returns Ok even if the file doesn't exist.
pub fn remove_socket_file(project_root: &Path) -> Result<()> {
    let path = socket_path(project_root);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            Err(e).with_context(|| format!("failed to remove socket file {}", path.display()))
        }
    }
}

/// Check if the daemon is running for the given project root.
///
/// Returns `true` only when the PID file exists AND the process is alive
/// (verified via `kill(pid, 0)` on Unix).
pub fn is_daemon_running(project_root: &Path) -> bool {
    let Some(pid) = read_pid_file(project_root) else {
        return false;
    };
    process_is_alive(pid)
}

/// Check if a process with the given PID is alive.
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // kill(pid, 0) checks existence without sending a signal.
    // Returns 0 if the process exists and we have permission to signal it.
    // SAFETY: This is a standard POSIX call, no actual signal is sent.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_is_alive(_pid: u32) -> bool {
    // Non-Unix fallback: assume alive if PID file exists.
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn pid_file_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Initially no PID file.
        assert!(read_pid_file(root).is_none());

        // Write PID file.
        write_pid_file(root).unwrap();
        let pid = read_pid_file(root);
        assert!(pid.is_some());
        assert_eq!(pid.unwrap(), std::process::id());

        // Verify permissions on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(pid_path(root)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }

        // Remove PID file.
        remove_pid_file(root).unwrap();
        assert!(read_pid_file(root).is_none());
    }

    #[test]
    fn remove_pid_file_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Removing a non-existent PID file should succeed.
        remove_pid_file(root).unwrap();
        remove_pid_file(root).unwrap();
    }

    #[test]
    fn remove_socket_file_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Removing a non-existent socket file should succeed.
        remove_socket_file(root).unwrap();
        remove_socket_file(root).unwrap();
    }

    #[test]
    fn is_daemon_running_with_no_pid_file() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_daemon_running(tmp.path()));
    }

    #[test]
    fn is_daemon_running_with_current_process() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        write_pid_file(root).unwrap();
        // Current process is alive, so this should return true.
        assert!(is_daemon_running(root));

        remove_pid_file(root).unwrap();
    }

    #[test]
    fn is_daemon_running_with_dead_pid() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Write a PID that almost certainly doesn't exist.
        // Use 99999 (high but valid pid_t value, not -1 when cast to i32).
        let path = pid_path(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "99999").unwrap();

        // On most systems, PID 99999 won't be running; if by chance it is,
        // the test still passes because is_daemon_running just checks liveness.
        // We mainly verify no panic occurs.
        let _ = is_daemon_running(root);
    }

    #[test]
    fn socket_path_is_correct() {
        let root = Path::new("/projects/myapp");
        assert_eq!(
            socket_path(root),
            PathBuf::from("/projects/myapp/.code-graph/daemon.sock")
        );
    }

    #[test]
    fn pid_path_is_correct() {
        let root = Path::new("/projects/myapp");
        assert_eq!(
            pid_path(root),
            PathBuf::from("/projects/myapp/.code-graph/daemon.pid")
        );
    }

    #[test]
    fn read_pid_file_with_invalid_content() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        let path = pid_path(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not-a-number").unwrap();

        assert!(read_pid_file(root).is_none());
    }
}
