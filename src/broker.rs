//! Broker daemon: HTTP server + SQLite for peer registry and message routing.

pub mod db;
pub mod routes;

use std::sync::Arc;
use std::time::Duration;

use error_stack::{Report, ResultExt as _};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use self::db::Database;
use self::routes::create_router;
use crate::process::is_process_alive;

#[derive(Debug, Error)]
#[error("broker error")]
pub struct BrokerRunError;

/// Start the Broker HTTP server on the given port.
pub async fn run_broker(port: u16) -> Result<(), Report<BrokerRunError>> {
    let db_path = std::env::temp_dir().join("sinew-broker.db");
    let db_path_str = db_path.to_string_lossy();
    info!(path = %db_path_str, "Initializing database");

    let db = Database::new(&db_path_str)
        .await
        .change_context(BrokerRunError)
        .attach("failed to initialize primary database")?;

    let db_for_cleanup = Arc::new(
        Database::new(&db_path_str)
            .await
            .change_context(BrokerRunError)
            .attach("failed to initialize cleanup database")?,
    );

    // Cancellation token for graceful shutdown (shared with /shutdown endpoint)
    let shutdown_token = CancellationToken::new();

    // Spawn stale peer cleanup task
    let cleanup_db = db_for_cleanup.clone();
    tokio::spawn(async move {
        stale_cleanup_loop(cleanup_db).await;
    });

    let app = create_router(db, shutdown_token.clone());

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .change_context(BrokerRunError)
        .attach_with(|| format!("failed to bind to {addr}"))?;
    info!(address = %addr, "Broker listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(graceful_shutdown(shutdown_token))
        .await
        .change_context(BrokerRunError)
        .attach("HTTP server error")?;

    info!("Broker shut down");
    Ok(())
}

/// Wait for either OS signals or the shutdown token to be cancelled.
async fn graceful_shutdown(token: CancellationToken) {
    tokio::select! {
        () = token.cancelled() => {
            info!("Shutdown requested via /shutdown endpoint");
        }
        () = os_shutdown_signal() => {
            info!("Shutdown requested via OS signal");
        }
    }
}

async fn os_shutdown_signal() {
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

/// Periodically remove peers whose processes are no longer alive.
async fn stale_cleanup_loop(db: Arc<Database>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;

        match db.get_all_peers().await {
            Ok(peers) => {
                let dead_ids: Vec<String> = peers
                    .iter()
                    .filter(|p| !is_process_alive(p.pid))
                    .map(|p| p.id.clone())
                    .collect();

                if !dead_ids.is_empty() {
                    info!(count = dead_ids.len(), "Removing stale peers");
                    if let Err(e) = db.remove_peers(&dead_ids).await {
                        error!(error = %e, "Failed to remove stale peers");
                    }
                }
            }
            Err(e) => {
                error!(error = %e, "Failed to fetch peers for cleanup");
            }
        }
    }
}
