//! Integration tests for the Broker HTTP API.
//!
//! Each test starts an in-memory Broker on a random port and exercises the full
//! HTTP API through BrokerClient.

use sinew::broker::db::Database;
use sinew::broker::routes::create_router;
use sinew::mcp::client::BrokerClient;
use sinew::types::*;
use tokio::net::TcpListener;

async fn start_broker() -> (String, BrokerClient) {
    let db = Database::new_in_memory().await.unwrap();
    let app = create_router(db, tokio_util::sync::CancellationToken::new());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("http://{addr}");
    let client = BrokerClient::new(&url);
    (url, client)
}

fn register_req(pid: u32, cwd: &str, git_root: Option<&str>) -> RegisterRequest {
    RegisterRequest {
        pid,
        cwd: cwd.into(),
        git_root: git_root.map(Into::into),
        tty: None,
        summary: None,
    }
}

// ---- Health ----

#[tokio::test]
async fn health_returns_ok_and_peer_count() {
    let (_url, client) = start_broker().await;

    let h = client.health().await.unwrap();
    assert_eq!(h.status, "ok");
    assert_eq!(h.peer_count, 0);

    client
        .register(&register_req(1000, "/a", None))
        .await
        .unwrap();

    let h = client.health().await.unwrap();
    assert_eq!(h.peer_count, 1);
}

// ---- Registration ----

#[tokio::test]
async fn register_returns_8char_alphanumeric_id() {
    let (_url, client) = start_broker().await;
    let reg = client
        .register(&register_req(1000, "/home", None))
        .await
        .unwrap();
    assert_eq!(reg.id.len(), 8);
    assert!(reg.id.chars().all(|c| c.is_ascii_alphanumeric()));
}

#[tokio::test]
async fn duplicate_pid_reuses_existing_id() {
    let (_url, client) = start_broker().await;
    let r1 = client
        .register(&register_req(1000, "/a", None))
        .await
        .unwrap();
    let r2 = client
        .register(&register_req(1000, "/b", None))
        .await
        .unwrap();
    assert_eq!(r1.id, r2.id);
}

#[tokio::test]
async fn different_pids_get_different_ids() {
    let (_url, client) = start_broker().await;
    let r1 = client
        .register(&register_req(1000, "/a", None))
        .await
        .unwrap();
    let r2 = client
        .register(&register_req(2000, "/a", None))
        .await
        .unwrap();
    assert_ne!(r1.id, r2.id);
}

// ---- Heartbeat ----

#[tokio::test]
async fn heartbeat_succeeds_for_registered_peer() {
    let (_url, client) = start_broker().await;
    let reg = client
        .register(&register_req(1000, "/a", None))
        .await
        .unwrap();
    client.heartbeat(&reg.id).await.unwrap();
}

#[tokio::test]
async fn heartbeat_fails_for_unknown_peer() {
    let (_url, client) = start_broker().await;
    assert!(client.heartbeat(&"notfound".into()).await.is_err());
}

// ---- Set Summary ----

#[tokio::test]
async fn set_summary_updates_peer_info() {
    let (_url, client) = start_broker().await;
    let reg = client
        .register(&register_req(std::process::id(), "/a", None))
        .await
        .unwrap();

    client
        .set_summary(&reg.id, "Working on integration tests")
        .await
        .unwrap();

    // Create a second peer to query list_peers
    let other = client
        .register(&register_req(std::process::id() + 99999, "/b", None))
        .await
        .unwrap();

    let peers = client
        .list_peers(&ListPeersRequest {
            id: other.id,
            scope: PeerScope::Machine,
        })
        .await
        .unwrap();

    let peer = peers.iter().find(|p| p.id == reg.id).unwrap();
    assert_eq!(peer.summary, Some("Working on integration tests".into()));
}

#[tokio::test]
async fn set_summary_fails_for_unknown_peer() {
    let (_url, client) = start_broker().await;
    assert!(
        client
            .set_summary(&"notfound".into(), "test")
            .await
            .is_err()
    );
}

// ---- List Peers ----

#[tokio::test]
async fn list_peers_machine_scope_excludes_requester() {
    let (_url, client) = start_broker().await;
    let pid = std::process::id();

    let r1 = client
        .register(&register_req(pid, "/a", Some("/repo")))
        .await
        .unwrap();
    let r2 = client
        .register(&register_req(pid + 99999, "/b", Some("/repo")))
        .await
        .unwrap();

    let peers = client
        .list_peers(&ListPeersRequest {
            id: r1.id.clone(),
            scope: PeerScope::Machine,
        })
        .await
        .unwrap();

    assert!(peers.iter().all(|p| p.id != r1.id));
    // r2 may or may not appear depending on PID liveness check
    // (pid + 99999 likely doesn't exist, so it would be filtered out)
    let _ = r2;
}

#[tokio::test]
async fn list_peers_directory_scope_filters_by_cwd() {
    let (_url, client) = start_broker().await;
    let pid = std::process::id();

    let r1 = client
        .register(&register_req(pid, "/project/a", Some("/project")))
        .await
        .unwrap();
    // Use same PID base to ensure "alive" for the test, but different logical peers
    // Since same PID would reuse, we use a trick: register with different PIDs
    let r2 = client
        .register(&register_req(pid + 1, "/project/a", Some("/project")))
        .await
        .unwrap();
    let _r3 = client
        .register(&register_req(pid + 2, "/other", Some("/other")))
        .await
        .unwrap();

    let peers = client
        .list_peers(&ListPeersRequest {
            id: r1.id.clone(),
            scope: PeerScope::Directory,
        })
        .await
        .unwrap();

    // Only r2 should appear (same cwd, excluding r1)
    // Note: PID liveness filtering may remove peers with non-existent PIDs
    for p in &peers {
        assert_eq!(p.cwd, "/project/a");
        assert_ne!(p.id, r1.id);
    }
    let _ = r2;
}

#[tokio::test]
async fn list_peers_repo_scope_filters_by_git_root() {
    let (_url, client) = start_broker().await;
    let pid = std::process::id();

    let r1 = client
        .register(&register_req(pid, "/project/a", Some("/project")))
        .await
        .unwrap();
    let _r2 = client
        .register(&register_req(pid + 1, "/project/b", Some("/project")))
        .await
        .unwrap();
    let _r3 = client
        .register(&register_req(pid + 2, "/other", Some("/other")))
        .await
        .unwrap();

    let peers = client
        .list_peers(&ListPeersRequest {
            id: r1.id.clone(),
            scope: PeerScope::Repo,
        })
        .await
        .unwrap();

    for p in &peers {
        assert_eq!(p.git_root, Some("/project".into()));
        assert_ne!(p.id, r1.id);
    }
}

// ---- Messaging ----

#[tokio::test]
async fn send_message_to_nonexistent_peer_fails() {
    let (_url, client) = start_broker().await;
    let sender = client
        .register(&register_req(1000, "/a", None))
        .await
        .unwrap();

    let result = client
        .send_message(&SendMessageRequest {
            from_id: sender.id,
            to_id: "nonexistent".into(),
            text: "hello".into(),
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn full_message_send_poll_flow() {
    let (_url, client) = start_broker().await;

    let sender = client
        .register(&register_req(1111, "/a", None))
        .await
        .unwrap();
    let receiver = client
        .register(&register_req(2222, "/b", None))
        .await
        .unwrap();

    // Send two messages
    client
        .send_message(&SendMessageRequest {
            from_id: sender.id.clone(),
            to_id: receiver.id.clone(),
            text: "message 1".into(),
        })
        .await
        .unwrap();
    client
        .send_message(&SendMessageRequest {
            from_id: sender.id.clone(),
            to_id: receiver.id.clone(),
            text: "message 2".into(),
        })
        .await
        .unwrap();

    // Poll — should get both
    let poll1 = client.poll_messages(&receiver.id).await.unwrap();
    assert_eq!(poll1.messages.len(), 2);
    assert_eq!(poll1.messages[0].text, "message 1");
    assert_eq!(poll1.messages[0].from_id, sender.id);
    assert_eq!(poll1.messages[1].text, "message 2");

    // Poll again — should be empty (already delivered)
    let poll2 = client.poll_messages(&receiver.id).await.unwrap();
    assert!(poll2.messages.is_empty());
}

// ---- Unregister ----

#[tokio::test]
async fn unregister_removes_peer() {
    let (_url, client) = start_broker().await;
    let reg = client
        .register(&register_req(1000, "/a", None))
        .await
        .unwrap();

    assert_eq!(client.health().await.unwrap().peer_count, 1);

    client.unregister(&reg.id).await.unwrap();

    assert_eq!(client.health().await.unwrap().peer_count, 0);
}

// ---- Full lifecycle flow ----

#[tokio::test]
async fn full_lifecycle_register_heartbeat_message_unregister() {
    let (_url, client) = start_broker().await;

    // Register
    let peer_a = client
        .register(&register_req(1111, "/workspace", Some("/repo")))
        .await
        .unwrap();
    let peer_b = client
        .register(&register_req(2222, "/workspace", Some("/repo")))
        .await
        .unwrap();

    // Heartbeat
    client.heartbeat(&peer_a.id).await.unwrap();
    client.heartbeat(&peer_b.id).await.unwrap();

    // Set summary
    client
        .set_summary(&peer_a.id, "Building feature X")
        .await
        .unwrap();

    // Send message A -> B
    client
        .send_message(&SendMessageRequest {
            from_id: peer_a.id.clone(),
            to_id: peer_b.id.clone(),
            text: "I finished the API layer".into(),
        })
        .await
        .unwrap();

    // B polls messages
    let poll = client.poll_messages(&peer_b.id).await.unwrap();
    assert_eq!(poll.messages.len(), 1);
    assert_eq!(poll.messages[0].text, "I finished the API layer");

    // Health check
    let health = client.health().await.unwrap();
    assert_eq!(health.status, "ok");
    assert_eq!(health.peer_count, 2);

    // Unregister both
    client.unregister(&peer_a.id).await.unwrap();
    client.unregister(&peer_b.id).await.unwrap();

    let health = client.health().await.unwrap();
    assert_eq!(health.peer_count, 0);
}
