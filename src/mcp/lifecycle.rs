//! MCP server lifecycle: startup, background tasks, recovery, and shutdown.

use std::io::IsTerminal;
use std::process::Command as StdCommand;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use error_stack::{Report, ResultExt};
use rmcp::service::{Peer, RoleServer};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::types::RegisterRequest;

use super::launcher;
use super::server::{McpState, StrandMcpServer};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const RECOVERY_THRESHOLD: u32 = 3;

#[derive(Debug, Error)]
#[error("MCP server error")]
pub struct McpError;

/// Run the full MCP server lifecycle.
pub async fn run_mcp_server(broker_url: &str) -> Result<(), Report<McpError>> {
    let port = extract_port(broker_url);

    // Ensure broker is running
    launcher::ensure_broker(broker_url, port)
        .await
        .change_context(McpError)
        .attach("failed to ensure broker is running")?;

    // Collect environment info
    let env_info = collect_env_info();
    info!(
        cwd = %env_info.cwd,
        git_root = ?env_info.git_root,
        tty = ?env_info.tty,
        "Collected environment info"
    );

    // Create MCP server
    let mcp_server = StrandMcpServer::new(broker_url);
    let state = mcp_server.state().clone();

    // Register with broker
    let reg = state
        .broker_client
        .register(&RegisterRequest {
            pid: std::process::id(),
            cwd: env_info.cwd,
            git_root: env_info.git_root,
            tty: env_info.tty,
            summary: None,
        })
        .await
        .change_context(McpError)
        .attach("failed to register with broker")?;

    let peer_id = reg.id;
    info!(peer_id = %peer_id, "Registered with broker");
    *state.peer_id.write().await = Some(peer_id.clone());

    // Start MCP server over stdio (Router wraps ServerHandler and handles tool routing)
    let router = mcp_server.into_router();
    let running = rmcp::ServiceExt::serve(router, rmcp::transport::stdio())
        .await
        .map_err(|e| Report::new(McpError).attach(format!("failed to start MCP server: {e}")))?;

    let peer_handle = running.peer().clone();
    let cancel = CancellationToken::new();
    let failure_count = Arc::new(AtomicU32::new(0));

    // Spawn background tasks
    let heartbeat_handle = tokio::spawn({
        let state = state.clone();
        let cancel = cancel.clone();
        let failure_count = failure_count.clone();
        let broker_url = broker_url.to_string();
        async move {
            heartbeat_loop(&state, &cancel, &failure_count, &broker_url, port).await;
        }
    });

    let poller_handle = tokio::spawn({
        let state = state.clone();
        let cancel = cancel.clone();
        let failure_count = failure_count.clone();
        let peer = peer_handle.clone();
        let broker_url = broker_url.to_string();
        async move {
            message_poll_loop(&state, &peer, &cancel, &failure_count, &broker_url, port).await;
        }
    });

    // Spawn shutdown signal handler
    let shutdown_cancel = cancel.clone();
    let shutdown_state = state.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        info!("Shutdown signal received");

        let peer_id = shutdown_state.peer_id.read().await;
        if let Some(id) = peer_id.as_ref() {
            if let Err(e) = shutdown_state.broker_client.unregister(id).await {
                warn!(error = %e, "Failed to unregister on shutdown");
            } else {
                info!("Unregistered from broker");
            }
        }

        shutdown_cancel.cancel();
    });

    // Wait for MCP server to complete (client disconnect)
    running
        .waiting()
        .await
        .change_context(McpError)
        .attach("MCP server runtime error")?;

    // Clean up background tasks
    cancel.cancel();
    if let Err(e) = heartbeat_handle.await {
        error!(error = %e, "Heartbeat task panicked");
    }
    if let Err(e) = poller_handle.await {
        error!(error = %e, "Poller task panicked");
    }

    info!("MCP server shut down");
    Ok(())
}

async fn heartbeat_loop(
    state: &McpState,
    cancel: &CancellationToken,
    failure_count: &AtomicU32,
    broker_url: &str,
    port: u16,
) {
    let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = interval.tick() => {}
        }

        let peer_id = state.peer_id.read().await;
        let Some(id) = peer_id.as_ref() else {
            continue;
        };

        match state.broker_client.heartbeat(id).await {
            Ok(()) => {
                failure_count.store(0, Ordering::Relaxed);
            }
            Err(e) => {
                let count = failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(error = %e, consecutive_failures = count, "Heartbeat failed");

                if count >= RECOVERY_THRESHOLD {
                    drop(peer_id);
                    attempt_recovery(state, broker_url, port).await;
                    failure_count.store(0, Ordering::Relaxed);
                }
            }
        }
    }
}

async fn message_poll_loop(
    state: &McpState,
    peer: &Peer<RoleServer>,
    cancel: &CancellationToken,
    failure_count: &AtomicU32,
    broker_url: &str,
    port: u16,
) {
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    loop {
        tokio::select! {
            () = cancel.cancelled() => break,
            _ = interval.tick() => {}
        }

        let peer_id = state.peer_id.read().await;
        let Some(id) = peer_id.as_ref() else {
            continue;
        };

        match state.broker_client.poll_messages(id).await {
            Ok(resp) => {
                failure_count.store(0, Ordering::Relaxed);

                for msg in &resp.messages {
                    let (from_summary, from_cwd) = lookup_peer_info(state, &msg.from_id).await;

                    if let Err(e) = StrandMcpServer::send_channel_notification(
                        peer,
                        &msg.text,
                        &msg.from_id,
                        &from_summary,
                        &from_cwd,
                        &msg.sent_at,
                    )
                    .await
                    {
                        error!(error = %e, "Failed to send channel notification");
                    }
                }
            }
            Err(e) => {
                let count = failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                warn!(error = %e, consecutive_failures = count, "Message poll failed");

                if count >= RECOVERY_THRESHOLD {
                    drop(peer_id);
                    attempt_recovery(state, broker_url, port).await;
                    failure_count.store(0, Ordering::Relaxed);
                }
            }
        }
    }
}

async fn attempt_recovery(state: &McpState, broker_url: &str, port: u16) {
    info!("Attempting broker recovery...");

    match launcher::ensure_broker(broker_url, port).await {
        Ok(()) => {
            info!("Broker recovered, re-registering...");

            let env_info = collect_env_info();
            match state
                .broker_client
                .register(&RegisterRequest {
                    pid: std::process::id(),
                    cwd: env_info.cwd,
                    git_root: env_info.git_root,
                    tty: env_info.tty,
                    summary: None,
                })
                .await
            {
                Ok(reg) => {
                    *state.peer_id.write().await = Some(reg.id.clone());
                    info!(peer_id = %reg.id, "Re-registered after recovery");
                }
                Err(e) => {
                    error!(error = %e, "Failed to re-register after recovery");
                }
            }
        }
        Err(e) => {
            error!(error = %e, "Broker recovery failed");
        }
    }
}

/// Look up a peer's summary and cwd for channel notification metadata.
async fn lookup_peer_info(state: &McpState, peer_id: &str) -> (String, String) {
    let req = crate::types::ListPeersRequest {
        id: "lookup".into(),
        scope: crate::types::PeerScope::Machine,
    };

    if let Ok(peers) = state.broker_client.list_peers(&req).await
        && let Some(peer) = peers.iter().find(|p| p.id == peer_id)
    {
        return (peer.summary.clone().unwrap_or_default(), peer.cwd.clone());
    }

    (String::new(), String::new())
}

struct EnvInfo {
    cwd: String,
    git_root: Option<String>,
    tty: Option<String>,
}

fn collect_env_info() -> EnvInfo {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to get current directory, using '.'");
            ".".into()
        });

    let git_root = StdCommand::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        });

    let tty = if std::io::stdin().is_terminal() {
        Some("interactive".into())
    } else {
        None
    };

    EnvInfo { cwd, git_root, tty }
}

fn extract_port(broker_url: &str) -> u16 {
    broker_url
        .rsplit(':')
        .next()
        .and_then(|s| s.trim_end_matches('/').parse().ok())
        .unwrap_or(7899)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            warn!(error = %e, "Failed to install Ctrl+C handler");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                warn!(error = %e, "Failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_port_from_url() {
        assert_eq!(extract_port("http://127.0.0.1:7899"), 7899);
        assert_eq!(extract_port("http://127.0.0.1:8080"), 8080);
        assert_eq!(extract_port("http://127.0.0.1:8080/"), 8080);
        assert_eq!(extract_port("http://localhost"), 7899);
    }
}
