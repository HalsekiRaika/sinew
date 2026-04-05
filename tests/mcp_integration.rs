//! Integration tests for multi-session MCP scenarios.
//!
//! These tests verify peer-to-peer interactions that go beyond single-peer
//! broker API tests: bidirectional messaging and scope-based discovery.

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

#[tokio::test]
async fn two_sessions_exchange_messages_bidirectionally() {
    let (_url, client) = start_broker().await;
    let pid = std::process::id();

    let session_a = client
        .register(&RegisterRequest {
            pid,
            cwd: "/project".into(),
            git_root: Some("/project".into()),
            tty: None,
            summary: Some("Working on backend".into()),
        })
        .await
        .unwrap();

    let session_b = client
        .register(&RegisterRequest {
            pid: pid + 1,
            cwd: "/project".into(),
            git_root: Some("/project".into()),
            tty: None,
            summary: Some("Working on frontend".into()),
        })
        .await
        .unwrap();

    // A -> B
    client
        .send_message(&SendMessageRequest {
            from_id: session_a.id.clone(),
            to_id: session_b.id.clone(),
            text: "I've pushed the API changes, please review".into(),
        })
        .await
        .unwrap();

    let poll = client.poll_messages(&session_b.id).await.unwrap();
    assert_eq!(poll.messages.len(), 1);
    assert_eq!(poll.messages[0].from_id, session_a.id);

    // B -> A (reply)
    client
        .send_message(&SendMessageRequest {
            from_id: session_b.id.clone(),
            to_id: session_a.id.clone(),
            text: "LGTM, merging now".into(),
        })
        .await
        .unwrap();

    let poll_a = client.poll_messages(&session_a.id).await.unwrap();
    assert_eq!(poll_a.messages.len(), 1);
    assert_eq!(poll_a.messages[0].text, "LGTM, merging now");
}

#[tokio::test]
async fn scope_filtering_separates_sessions_by_repo_and_directory() {
    let (_url, client) = start_broker().await;
    let pid = std::process::id();

    // Two sessions in same repo but different directories, one in a different repo
    let s1 = client
        .register(&RegisterRequest {
            pid,
            cwd: "/project/frontend".into(),
            git_root: Some("/project".into()),
            tty: None,
            summary: None,
        })
        .await
        .unwrap();

    let s2 = client
        .register(&RegisterRequest {
            pid: pid + 1,
            cwd: "/project/backend".into(),
            git_root: Some("/project".into()),
            tty: None,
            summary: None,
        })
        .await
        .unwrap();

    let _s3 = client
        .register(&RegisterRequest {
            pid: pid + 2,
            cwd: "/other-project".into(),
            git_root: Some("/other-project".into()),
            tty: None,
            summary: None,
        })
        .await
        .unwrap();

    // Machine scope: sees everyone else
    let machine_peers = client
        .list_peers(&ListPeersRequest {
            id: s1.id.clone(),
            scope: PeerScope::Machine,
        })
        .await
        .unwrap();
    assert!(machine_peers.iter().all(|p| p.id != s1.id));

    // Directory scope: nobody shares s1's cwd
    let dir_peers = client
        .list_peers(&ListPeersRequest {
            id: s1.id.clone(),
            scope: PeerScope::Directory,
        })
        .await
        .unwrap();
    for p in &dir_peers {
        assert_eq!(p.cwd, "/project/frontend");
    }

    // Repo scope: only s2 shares the git root
    let repo_peers = client
        .list_peers(&ListPeersRequest {
            id: s1.id.clone(),
            scope: PeerScope::Repo,
        })
        .await
        .unwrap();
    for p in &repo_peers {
        assert_eq!(p.git_root, Some("/project".into()));
        assert_ne!(p.id, s1.id);
    }

    let _ = s2;
}
