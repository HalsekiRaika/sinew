//! HTTP client for communicating with the Broker daemon.

use reqwest::Client;
use thiserror::Error;

use crate::types::{
    HealthResponse, HeartbeatRequest, ListPeersRequest, Peer, PeerId, PollMessagesResponse,
    RegisterRequest, RegisterResponse, SendMessageRequest, SetSummaryRequest, UnregisterRequest,
};

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),

    #[error("broker returned error {status}: {body}")]
    BrokerError { status: u16, body: String },
}

pub struct BrokerClient {
    client: Client,
    base_url: String,
}

impl BrokerClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn register(&self, req: &RegisterRequest) -> Result<RegisterResponse, ClientError> {
        let resp = self
            .client
            .post(format!("{}/register", self.base_url))
            .json(req)
            .send()
            .await?;
        Self::check_status(&resp)?;
        Ok(resp.json().await?)
    }

    pub async fn heartbeat(&self, id: &PeerId) -> Result<(), ClientError> {
        let resp = self
            .client
            .post(format!("{}/heartbeat", self.base_url))
            .json(&HeartbeatRequest { id: id.clone() })
            .send()
            .await?;
        Self::check_status(&resp)?;
        Ok(())
    }

    pub async fn set_summary(&self, id: &PeerId, summary: &str) -> Result<(), ClientError> {
        let resp = self
            .client
            .post(format!("{}/set-summary", self.base_url))
            .json(&SetSummaryRequest {
                id: id.clone(),
                summary: summary.to_string(),
            })
            .send()
            .await?;
        Self::check_status(&resp)?;
        Ok(())
    }

    pub async fn list_peers(&self, req: &ListPeersRequest) -> Result<Vec<Peer>, ClientError> {
        let resp = self
            .client
            .post(format!("{}/list-peers", self.base_url))
            .json(req)
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn send_message(&self, req: &SendMessageRequest) -> Result<(), ClientError> {
        let resp = self
            .client
            .post(format!("{}/send-message", self.base_url))
            .json(req)
            .send()
            .await?;
        Self::check_status(&resp)?;
        Ok(())
    }

    pub async fn poll_messages(&self, id: &PeerId) -> Result<PollMessagesResponse, ClientError> {
        let resp = self
            .client
            .post(format!("{}/poll-messages", self.base_url))
            .json(&crate::types::PollMessagesRequest { id: id.clone() })
            .send()
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn shutdown(&self) -> Result<(), ClientError> {
        let resp = self
            .client
            .post(format!("{}/shutdown", self.base_url))
            .send()
            .await?;
        Self::check_status(&resp)?;
        Ok(())
    }

    pub async fn unregister(&self, id: &PeerId) -> Result<(), ClientError> {
        let resp = self
            .client
            .post(format!("{}/unregister", self.base_url))
            .json(&UnregisterRequest { id: id.clone() })
            .send()
            .await?;
        Self::check_status(&resp)?;
        Ok(())
    }

    fn check_status(resp: &reqwest::Response) -> Result<(), ClientError> {
        let status = resp.status();
        if status.is_client_error() || status.is_server_error() {
            return Err(ClientError::BrokerError {
                status: status.as_u16(),
                body: format!("HTTP {status}"),
            });
        }
        Ok(())
    }
}
