use std::net::SocketAddr;

use proto::downloader::{
    node_capabilities_client::NodeCapabilitiesClient, node_cookie_manager_client::NodeCookieManagerClient, Empty, PushCookieRequest,
    RemoveCookieRequest,
};
use tonic::transport::Channel;

use crate::{
    authenticated_request,
    client::{DownloaderTlsConfig, NodeClient},
};

#[derive(Clone)]
pub struct AssignmentNodeClient {
    client: NodeClient,
    cookie_manager_token: Box<str>,
}

impl AssignmentNodeClient {
    #[must_use]
    pub fn load(config: &DownloaderTlsConfig, server_name: &str, cookie_manager_token: Box<str>) -> Self {
        Self {
            client: NodeClient::load(config, server_name),
            cookie_manager_token,
        }
    }

    /// Builds a cookie-assignment RPC handle for one downloader node address.
    ///
    /// # Errors
    ///
    /// Returns a transport error if the gRPC channel cannot be built.
    pub fn build_handle(&self, address: SocketAddr) -> Result<AssignmentNodeHandle, tonic::transport::Error> {
        let address = format!("https://{address}");
        let channel = self.client.build_channel(&address)?;

        Ok(AssignmentNodeHandle {
            address: address.into_boxed_str(),
            cookie_manager_token: self.cookie_manager_token.clone(),
            channel,
        })
    }
}

#[derive(Clone)]
pub struct AssignmentNodeHandle {
    pub address: Box<str>,
    cookie_manager_token: Box<str>,
    channel: Channel,
}

impl AssignmentNodeHandle {
    /// Checks that the worker is reachable through the capabilities service.
    ///
    /// # Errors
    ///
    /// Returns an error if auth metadata cannot be built or the RPC fails.
    pub async fn fetch_status(&self) -> Result<(), AssignmentNodeHandleError> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        client
            .get_status(authenticated_request(Empty {}, &self.cookie_manager_token)?)
            .await?;
        Ok(())
    }

    /// Lists cookie identifiers currently stored on the worker.
    ///
    /// # Errors
    ///
    /// Returns an error if auth metadata cannot be built or the RPC fails.
    pub async fn list_node_cookies(&self) -> Result<Vec<String>, AssignmentNodeHandleError> {
        let mut client = NodeCookieManagerClient::new(self.channel.clone());
        let response = client
            .list_node_cookies(authenticated_request(Empty {}, &self.cookie_manager_token)?)
            .await?;
        Ok(response.into_inner().cookie_ids)
    }

    /// Pushes cookie data to the worker.
    ///
    /// # Errors
    ///
    /// Returns an error if auth metadata cannot be built or the RPC fails.
    pub async fn push_cookie(&self, cookie_id: &str, domain: &str, data: &str) -> Result<(), AssignmentNodeHandleError> {
        let mut client = NodeCookieManagerClient::new(self.channel.clone());
        client
            .push_cookie(authenticated_request(
                PushCookieRequest {
                    cookie_id: cookie_id.to_owned(),
                    domain: domain.to_owned(),
                    data: data.to_owned(),
                },
                &self.cookie_manager_token,
            )?)
            .await?;
        Ok(())
    }

    /// Removes a cookie from the worker.
    ///
    /// # Errors
    ///
    /// Returns an error if auth metadata cannot be built or the RPC fails.
    pub async fn remove_cookie(&self, cookie_id: &str) -> Result<(), AssignmentNodeHandleError> {
        let mut client = NodeCookieManagerClient::new(self.channel.clone());
        client
            .remove_cookie(authenticated_request(
                RemoveCookieRequest {
                    cookie_id: cookie_id.to_owned(),
                },
                &self.cookie_manager_token,
            )?)
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AssignmentNodeHandleError {
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
}
