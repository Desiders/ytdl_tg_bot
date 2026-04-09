use std::{collections::HashSet, env, fs, io, net::SocketAddr};

use tonic::{
    metadata::{Ascii, MetadataValue},
    transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity},
};
use url::Url;
use ytdl_tg_bot_proto::downloader::{
    node_capabilities_client::NodeCapabilitiesClient, node_cookie_manager_client::NodeCookieManagerClient, Empty, PushCookieRequest,
    RemoveCookieRequest,
};

use crate::config::DownloadTlsConfig;

#[derive(Clone)]
pub struct DownloaderServiceTarget {
    pub host: Box<str>,
    pub port: u16,
}

impl DownloaderServiceTarget {
    pub fn from_env() -> Self {
        let raw = env::var("DOWNLOADER_SERVICE_DNS").expect("Missing required env var `DOWNLOADER_SERVICE_DNS`");
        let (host, port) = parse_host(&raw);

        Self {
            host: host.into_boxed_str(),
            port,
        }
    }

    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub async fn resolve_nodes(&self) -> io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host(self.authority())
            .await?
            .collect::<HashSet<_>>()
            .into_iter()
            .collect())
    }
}

#[derive(Clone)]
pub struct NodeClient {
    ca_cert_pem: Box<[u8]>,
    cert_pem: Box<[u8]>,
    key_pem: Box<[u8]>,
    server_name: Box<str>,
}

impl NodeClient {
    pub fn load(config: &DownloadTlsConfig, server_name: &str) -> Self {
        let ca_cert_pem = fs::read(&*config.ca_cert_path).expect(&format!("Failed to read downloader CA certificate {}", config.cert_path));
        let cert_pem = fs::read(&*config.cert_path).expect(&format!("Failed to read cookie assignment certificate {}", config.cert_path));
        let key_pem = fs::read(&*config.key_path).expect(&format!("Failed to read cookie assignment key {}", config.key_path));
        Self {
            ca_cert_pem: ca_cert_pem.into(),
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            server_name: server_name.into(),
        }
    }

    pub fn build_handle(&self, address: SocketAddr, token: &str) -> Result<NodeHandle, tonic::transport::Error> {
        let address = format!("https://{address}");
        let tls_cfg = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(self.ca_cert_pem.as_ref()))
            .identity(Identity::from_pem(self.cert_pem.as_ref(), self.key_pem.as_ref()))
            .domain_name(self.server_name.as_ref());
        let endpoint = Endpoint::from_shared(address.clone())?.tls_config(tls_cfg)?;
        let channel = endpoint.connect_lazy();

        Ok(NodeHandle {
            address: address.into_boxed_str(),
            token: token.to_owned().into_boxed_str(),
            channel,
        })
    }
}

#[derive(Clone)]
pub struct NodeHandle {
    pub address: Box<str>,
    token: Box<str>,
    channel: Channel,
}

impl NodeHandle {
    pub async fn fetch_status(&self) -> Result<(), NodeHandleError> {
        let mut client = NodeCapabilitiesClient::new(self.channel.clone());
        client.get_status(authenticated_request(Empty {}, &self.token)?).await?;
        Ok(())
    }

    pub async fn list_node_cookies(&self) -> Result<Vec<String>, NodeHandleError> {
        let mut client = NodeCookieManagerClient::new(self.channel.clone());
        let response = client.list_node_cookies(authenticated_request(Empty {}, &self.token)?).await?;
        Ok(response.into_inner().cookie_ids)
    }

    pub async fn push_cookie(&self, cookie_id: &str, domain: &str, data: &str) -> Result<(), NodeHandleError> {
        let mut client = NodeCookieManagerClient::new(self.channel.clone());
        client
            .push_cookie(authenticated_request(
                PushCookieRequest {
                    cookie_id: cookie_id.to_owned(),
                    domain: domain.to_owned(),
                    data: data.to_owned(),
                },
                &self.token,
            )?)
            .await?;
        Ok(())
    }

    pub async fn remove_cookie(&self, cookie_id: &str) -> Result<(), NodeHandleError> {
        let mut client = NodeCookieManagerClient::new(self.channel.clone());
        client
            .remove_cookie(authenticated_request(
                RemoveCookieRequest {
                    cookie_id: cookie_id.to_owned(),
                },
                &self.token,
            )?)
            .await?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeHandleError {
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
}

fn authenticated_request<T>(message: T, token: &str) -> Result<tonic::Request<T>, tonic::metadata::errors::InvalidMetadataValue> {
    let mut request = tonic::Request::new(message);
    let value: MetadataValue<Ascii> = format!("Bearer {token}").parse()?;
    request.metadata_mut().insert("authorization", value);
    Ok(request)
}

fn parse_host(raw: &str) -> (String, u16) {
    let url = Url::parse(&format!("http://{raw}")).expect("URL must be valid");
    let host = url.host_str().expect("Host must be present");
    let port = url.port().expect("Port must be present");
    (host.to_owned(), port)
}

#[cfg(test)]
mod tests {
    use super::parse_host;

    #[test]
    fn parse_host_port_domain() {
        let (host, port) = parse_host("downloader.downloader.svc.cluster.local:50051");

        assert_eq!(host, "downloader.downloader.svc.cluster.local");
        assert_eq!(port, 50051);
    }

    #[test]
    fn parse_host_port_ipv6() {
        let (host, port) = parse_host("[::1]:50051");

        assert_eq!(host, "[::1]");
        assert_eq!(port, 50051);
    }
}
