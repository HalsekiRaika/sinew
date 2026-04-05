//! MCP server implementation using rmcp.

use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::handler::server::wrapper::{Json as ToolJson, Parameters};
use rmcp::model::{
    CustomNotification, Implementation, ServerCapabilities, ServerInfo, ServerNotification,
};
use rmcp::schemars;
use rmcp::service::{Peer, RoleServer};
use rmcp::tool;
use rmcp::tool_router;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::RwLock;
use tracing::error;

use crate::types::{ListPeersRequest, PeerId, PeerScope, SendMessageRequest};

use super::client::BrokerClient;

/// Shared state for the MCP server.
pub struct McpState {
    pub broker_client: BrokerClient,
    pub peer_id: RwLock<Option<PeerId>>,
}

pub struct StrandMcpServer {
    pub state: Arc<McpState>,
}

impl StrandMcpServer {
    pub fn new(broker_url: &str) -> Self {
        Self {
            state: Arc::new(McpState {
                broker_client: BrokerClient::new(broker_url),
                peer_id: RwLock::new(None),
            }),
        }
    }

    pub fn state(&self) -> &Arc<McpState> {
        &self.state
    }

    /// Build a Router that wraps this server with tool routes registered.
    /// The Router implements `Service<RoleServer>` and handles tool list/call automatically.
    pub fn into_router(self) -> rmcp::handler::server::router::Router<Self> {
        let mut router = rmcp::handler::server::router::Router::new(self);
        router.tool_router = Self::tool_router();
        router
    }

    /// Send a `notifications/claude/channel` notification to the connected client.
    /// Format matches the original claude-peers-mcp implementation.
    pub async fn send_channel_notification(
        peer: &Peer<RoleServer>,
        text: &str,
        from_id: &str,
        from_summary: &str,
        from_cwd: &str,
        sent_at: &str,
    ) -> Result<(), rmcp::service::ServiceError> {
        peer.send_notification(ServerNotification::CustomNotification(
            CustomNotification::new(
                "notifications/claude/channel",
                Some(json!({
                    "content": text,
                    "meta": {
                        "from_id": from_id,
                        "from_summary": from_summary,
                        "from_cwd": from_cwd,
                        "sent_at": sent_at,
                    }
                })),
            ),
        ))
        .await
    }
}

// Tool parameter/output types

#[derive(Deserialize, schemars::JsonSchema, Default)]
struct ListPeersParam {
    /// Scope: machine, directory, or repo
    scope: String,
}

#[derive(Serialize, schemars::JsonSchema)]
struct ToolTextOutput {
    text: String,
}

#[derive(Deserialize, schemars::JsonSchema, Default)]
struct SendMessageParam {
    /// Target peer ID
    to_id: String,
    /// Message text
    message: String,
}

#[derive(Deserialize, schemars::JsonSchema, Default)]
struct SetSummaryParam {
    /// Summary text describing current work
    summary: String,
}

#[tool_router]
impl StrandMcpServer {
    #[tool(name = "list_peers", description = "List peer Claude Code sessions")]
    async fn list_peers(
        &self,
        Parameters(param): Parameters<ListPeersParam>,
    ) -> ToolJson<ToolTextOutput> {
        let peer_id = self.state.peer_id.read().await;
        let Some(id) = peer_id.as_ref() else {
            return ToolJson(ToolTextOutput {
                text: "Error: not registered".into(),
            });
        };

        let scope = match param.scope.as_str() {
            "machine" => PeerScope::Machine,
            "directory" => PeerScope::Directory,
            "repo" => PeerScope::Repo,
            _ => {
                return ToolJson(ToolTextOutput {
                    text: "Invalid scope. Use: machine, directory, or repo".into(),
                });
            }
        };

        match self
            .state
            .broker_client
            .list_peers(&ListPeersRequest {
                id: id.clone(),
                scope,
            })
            .await
        {
            Ok(peers) => {
                let text = serde_json::to_string_pretty(&peers).unwrap_or_else(|_| "[]".into());
                ToolJson(ToolTextOutput { text })
            }
            Err(e) => {
                error!(error = %e, "list_peers failed");
                ToolJson(ToolTextOutput {
                    text: format!("Failed to list peers: {e}"),
                })
            }
        }
    }

    #[tool(
        name = "send_message",
        description = "Send a message to a peer Claude Code session"
    )]
    async fn send_message(
        &self,
        Parameters(param): Parameters<SendMessageParam>,
    ) -> ToolJson<ToolTextOutput> {
        let peer_id = self.state.peer_id.read().await;
        let Some(from_id) = peer_id.as_ref() else {
            return ToolJson(ToolTextOutput {
                text: "Error: not registered".into(),
            });
        };

        match self
            .state
            .broker_client
            .send_message(&SendMessageRequest {
                from_id: from_id.clone(),
                to_id: param.to_id,
                text: param.message,
            })
            .await
        {
            Ok(()) => ToolJson(ToolTextOutput {
                text: "Message sent".into(),
            }),
            Err(e) => {
                error!(error = %e, "send_message failed");
                ToolJson(ToolTextOutput {
                    text: format!("Failed to send message: {e}"),
                })
            }
        }
    }

    #[tool(
        name = "set_summary",
        description = "Set your session work summary visible to other peers"
    )]
    async fn set_summary(
        &self,
        Parameters(param): Parameters<SetSummaryParam>,
    ) -> ToolJson<ToolTextOutput> {
        let peer_id = self.state.peer_id.read().await;
        let Some(id) = peer_id.as_ref() else {
            return ToolJson(ToolTextOutput {
                text: "Error: not registered".into(),
            });
        };

        match self
            .state
            .broker_client
            .set_summary(id, &param.summary)
            .await
        {
            Ok(()) => ToolJson(ToolTextOutput {
                text: "Summary updated".into(),
            }),
            Err(e) => {
                error!(error = %e, "set_summary failed");
                ToolJson(ToolTextOutput {
                    text: format!("Failed to set summary: {e}"),
                })
            }
        }
    }

    #[tool(
        name = "check_messages",
        description = "Check for new messages from other Claude Code sessions"
    )]
    async fn check_messages(&self) -> ToolJson<ToolTextOutput> {
        let peer_id = self.state.peer_id.read().await;
        let Some(id) = peer_id.as_ref() else {
            return ToolJson(ToolTextOutput {
                text: "Error: not registered".into(),
            });
        };

        match self.state.broker_client.poll_messages(id).await {
            Ok(resp) => {
                if resp.messages.is_empty() {
                    ToolJson(ToolTextOutput {
                        text: "No new messages".into(),
                    })
                } else {
                    let text = serde_json::to_string_pretty(&resp.messages)
                        .unwrap_or_else(|_| "[]".into());
                    ToolJson(ToolTextOutput { text })
                }
            }
            Err(e) => {
                error!(error = %e, "check_messages failed");
                ToolJson(ToolTextOutput {
                    text: format!("Failed to check messages: {e}"),
                })
            }
        }
    }
}

impl ServerHandler for StrandMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::builder()
            .enable_tools()
            .enable_experimental()
            .build();

        // Set claude/channel in experimental capabilities
        if let Some(ref mut exp) = caps.experimental {
            exp.insert("claude/channel".into(), serde_json::Map::new());
        }

        ServerInfo::new(caps)
            .with_server_info(Implementation::new("sinew", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Peer discovery and messaging for Claude Code sessions. \
                 Use list_peers to find other sessions, send_message to communicate, \
                 set_summary to share what you're working on.",
            )
    }
}
