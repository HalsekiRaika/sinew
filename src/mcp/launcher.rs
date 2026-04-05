//! Broker process launcher: detached spawn and health-check wait.

use std::process::{Command, Stdio};
use std::time::Duration;

use thiserror::Error;
use tracing::info;

use super::client::BrokerClient;

#[derive(Debug, Error)]
pub enum LauncherError {
    #[error("failed to get current executable path: {0}")]
    CurrentExe(std::io::Error),

    #[error("failed to spawn broker process: {0}")]
    Spawn(std::io::Error),

    #[error("broker did not become healthy within timeout")]
    Timeout,
}

/// Ensure the broker is running. If not, spawn it as a detached process
/// and wait for it to become healthy.
pub async fn ensure_broker(broker_url: &str, port: u16) -> Result<(), LauncherError> {
    let client = BrokerClient::new(broker_url);

    // Check if already running
    if client.health().await.is_ok() {
        return Ok(());
    }

    info!("Broker not running, spawning...");
    spawn_detached_broker(port)?;

    // Wait up to 6 seconds for broker to become healthy
    for _ in 0..12 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        if client.health().await.is_ok() {
            info!("Broker is now healthy");
            return Ok(());
        }
    }

    Err(LauncherError::Timeout)
}

fn spawn_detached_broker(port: u16) -> Result<u32, LauncherError> {
    let exe = std::env::current_exe().map_err(LauncherError::CurrentExe)?;

    let mut cmd = Command::new(exe);
    cmd.args(["broker", "--port", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
        // Safety: setsid() is async-signal-safe
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid()
                    .map(|_| ())
                    .map_err(|e| std::io::Error::other(e.to_string()))
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd.spawn().map_err(LauncherError::Spawn)?;
    let pid = child.id();
    info!(pid = pid, "Spawned broker process");
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_detached_broker_gets_pid() {
        // This test verifies that we can spawn the binary with broker subcommand.
        // The spawned process will likely fail (port may be in use) but we verify
        // the spawn mechanism itself works.
        // We use a high port to avoid conflicts.
        let result = spawn_detached_broker(19876);
        // On CI or in tests, the binary should exist since we're running from it
        match result {
            Ok(pid) => assert!(pid > 0),
            Err(LauncherError::CurrentExe(_)) => {
                // Acceptable in some test environments
            }
            Err(e) => panic!("Unexpected error: {e}"),
        }
    }

    #[tokio::test]
    async fn ensure_broker_with_running_broker() {
        // Start a test broker in-process
        let db = crate::broker::db::Database::new_in_memory().await.unwrap();
        let app =
            crate::broker::routes::create_router(db, tokio_util::sync::CancellationToken::new());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let url = format!("http://{addr}");
        // ensure_broker should return immediately since broker is already running
        let result = ensure_broker(&url, addr.port()).await;
        assert!(result.is_ok());
    }
}
