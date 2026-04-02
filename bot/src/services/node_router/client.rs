use std::{env, fs};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use url::Url;

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
}

#[derive(Clone)]
pub(super) struct NodeClient {
    ca_cert_pem: Box<[u8]>,
    cert_pem: Box<[u8]>,
    key_pem: Box<[u8]>,
    server_name: Box<str>,
}

impl NodeClient {
    pub(super) fn load(config: &DownloadTlsConfig, server_name: &str) -> Self {
        let ca_cert_pem = fs::read(&*config.ca_cert_path).expect(&format!("Failed to read downloader CA certificate {}", config.cert_path));
        let cert_pem = fs::read(&*config.cert_path).expect(&format!("Failed to read bot certificate {}", config.cert_path));
        let key_pem = fs::read(&*config.key_path).expect(&format!("Failed to read bot key {}", config.key_path));
        Self {
            ca_cert_pem: ca_cert_pem.into(),
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            server_name: server_name.into(),
        }
    }

    pub(super) fn build_channel(&self, address: &str) -> Result<Channel, tonic::transport::Error> {
        let tls_cfg = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(self.ca_cert_pem.as_ref()))
            .identity(Identity::from_pem(self.cert_pem.as_ref(), self.key_pem.as_ref()))
            .domain_name(self.server_name.as_ref());
        let endpoint = Endpoint::from_shared(address.to_owned())?.tls_config(tls_cfg)?;
        Ok(endpoint.connect_lazy())
    }
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
