//! HTTP route handlers for the Broker API.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::types::{
    ErrorResponse, HealthResponse, HeartbeatRequest, ListPeersRequest, PollMessagesRequest,
    PollMessagesResponse, RegisterRequest, RegisterResponse, SendMessageRequest, SetSummaryRequest,
    UnregisterRequest,
};

use super::db::{Database, DbError};

#[derive(Clone)]
pub struct BrokerState {
    pub db: Arc<Database>,
    pub shutdown_token: CancellationToken,
}

fn internal_error(e: &DbError, context: &str) -> impl IntoResponse {
    error!(error = %e, context, "Database error in route handler");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: format!("internal error: {context}"),
        }),
    )
}

fn not_found_error(detail: &str) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: detail.to_string(),
        }),
    )
}

pub fn create_router(db: Database, shutdown_token: CancellationToken) -> Router {
    let state = BrokerState {
        db: Arc::new(db),
        shutdown_token,
    };

    Router::new()
        .route("/health", get(health))
        .route("/register", post(register))
        .route("/heartbeat", post(heartbeat))
        .route("/set-summary", post(set_summary))
        .route("/list-peers", post(list_peers))
        .route("/send-message", post(send_message))
        .route("/poll-messages", post(poll_messages))
        .route("/unregister", post(unregister))
        .route("/shutdown", post(shutdown))
        .with_state(state)
}

async fn shutdown(State(state): State<BrokerState>) -> impl IntoResponse {
    info!("Shutdown requested via API");
    state.shutdown_token.cancel();
    Json(serde_json::json!({"status": "shutting down"}))
}

async fn health(State(state): State<BrokerState>) -> impl IntoResponse {
    match state.db.get_all_peers().await {
        Ok(peers) => Json(HealthResponse {
            status: "ok".into(),
            peer_count: peers.len() as i64,
        })
        .into_response(),
        Err(e) => internal_error(&e, "health check").into_response(),
    }
}

async fn register(
    State(state): State<BrokerState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    match state.db.register_peer(&req).await {
        Ok(id) => (StatusCode::OK, Json(RegisterResponse { id })).into_response(),
        Err(e) => internal_error(&e, "register peer").into_response(),
    }
}

async fn heartbeat(
    State(state): State<BrokerState>,
    Json(req): Json<HeartbeatRequest>,
) -> impl IntoResponse {
    match state.db.update_heartbeat(&req.id).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(DbError::PeerNotFound(id)) => {
            not_found_error(&format!("peer not found: {id}")).into_response()
        }
        Err(e) => internal_error(&e, "heartbeat").into_response(),
    }
}

async fn set_summary(
    State(state): State<BrokerState>,
    Json(req): Json<SetSummaryRequest>,
) -> impl IntoResponse {
    match state.db.set_summary(&req.id, &req.summary).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(DbError::PeerNotFound(id)) => {
            not_found_error(&format!("peer not found: {id}")).into_response()
        }
        Err(e) => internal_error(&e, "set summary").into_response(),
    }
}

async fn list_peers(
    State(state): State<BrokerState>,
    Json(req): Json<ListPeersRequest>,
) -> impl IntoResponse {
    let all_peers = match state.db.get_all_peers().await {
        Ok(p) => p,
        Err(e) => return internal_error(&e, "list peers: fetch all").into_response(),
    };

    let requesting_peer = all_peers.iter().find(|p| p.id == req.id);
    let (ref_cwd, ref_git_root) = match requesting_peer {
        Some(p) => (p.cwd.as_str(), p.git_root.as_deref()),
        None => {
            return Json(Vec::<crate::types::Peer>::new()).into_response();
        }
    };

    let mut peers = match state
        .db
        .list_peers(&req.scope, &req.id, ref_cwd, ref_git_root)
        .await
    {
        Ok(p) => p,
        Err(e) => return internal_error(&e, "list peers: filter").into_response(),
    };

    // Check liveness and remove dead peers
    let dead_ids: Vec<String> = peers
        .iter()
        .filter(|p| !crate::process::is_process_alive(p.pid))
        .map(|p| p.id.clone())
        .collect();

    if !dead_ids.is_empty() {
        if let Err(e) = state.db.remove_peers(&dead_ids).await {
            error!(error = %e, count = dead_ids.len(), "Failed to remove dead peers during list");
        }
        peers.retain(|p| !dead_ids.contains(&p.id));
    }

    Json(peers).into_response()
}

async fn send_message(
    State(state): State<BrokerState>,
    Json(req): Json<SendMessageRequest>,
) -> impl IntoResponse {
    match state.db.peer_exists(&req.to_id).await {
        Ok(true) => {}
        Ok(false) => {
            return not_found_error(&format!("peer not found: {}", req.to_id)).into_response();
        }
        Err(e) => return internal_error(&e, "send message: check peer").into_response(),
    }

    match state.db.send_message(&req).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(e) => internal_error(&e, "send message: insert").into_response(),
    }
}

async fn poll_messages(
    State(state): State<BrokerState>,
    Json(req): Json<PollMessagesRequest>,
) -> impl IntoResponse {
    match state.db.poll_messages(&req.id).await {
        Ok(messages) => Json(PollMessagesResponse { messages }).into_response(),
        Err(e) => internal_error(&e, "poll messages").into_response(),
    }
}

async fn unregister(
    State(state): State<BrokerState>,
    Json(req): Json<UnregisterRequest>,
) -> impl IntoResponse {
    match state.db.unregister_peer(&req.id).await {
        Ok(()) => StatusCode::OK.into_response(),
        Err(e) => internal_error(&e, "unregister").into_response(),
    }
}
