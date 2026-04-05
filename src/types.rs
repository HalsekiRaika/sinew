//! Shared data types for Broker and MCP Server communication.

use serde::{Deserialize, Serialize};

/// 8-character alphanumeric peer identifier.
pub type PeerId = String;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub id: PeerId,
    pub pid: u32,
    pub cwd: String,
    pub git_root: Option<String>,
    pub tty: Option<String>,
    pub summary: Option<String>,
    pub registered_at: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub from_id: PeerId,
    pub to_id: PeerId,
    pub text: String,
    pub sent_at: String,
    pub delivered: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub pid: u32,
    pub cwd: String,
    pub git_root: Option<String>,
    pub tty: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListPeersRequest {
    pub id: PeerId,
    pub scope: PeerScope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PeerScope {
    Machine,
    Directory,
    Repo,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SendMessageRequest {
    pub from_id: PeerId,
    pub to_id: PeerId,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollMessagesRequest {
    pub id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollMessagesResponse {
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetSummaryRequest {
    pub id: PeerId,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnregisterRequest {
    pub id: PeerId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub peer_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_scope_serializes_as_lowercase_for_api_compatibility() {
        assert_eq!(
            serde_json::to_string(&PeerScope::Machine).unwrap(),
            r#""machine""#
        );
        assert_eq!(
            serde_json::to_string(&PeerScope::Directory).unwrap(),
            r#""directory""#
        );
        assert_eq!(
            serde_json::to_string(&PeerScope::Repo).unwrap(),
            r#""repo""#
        );

        let scope: PeerScope = serde_json::from_str(r#""machine""#).unwrap();
        assert_eq!(scope, PeerScope::Machine);
    }
}
