use std::{collections::HashSet, env, fs, io, net::SocketAddr};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use url::Url;

#[derive(Clone, Debug)]
pub struct DownloaderTlsConfig {
    pub ca_cert: Box<str>,
    pub cert: Box<str>,
    pub key: Box<str>,
}

#[derive(Clone, Debug)]
pub struct DownloaderServiceTarget {
    pub host: Box<str>,
    pub port: u16,
}

impl DownloaderServiceTarget {
    /// Builds a downloader service target from `DOWNLOADER_SERVICE_DNS`.
    ///
    /// # Panics
    ///
    /// Panics if `DOWNLOADER_SERVICE_DNS` is missing or is not a host:port
    /// authority.
    #[must_use]
    pub fn from_env() -> Self {
        let raw = env::var("DOWNLOADER_SERVICE_DNS").expect("Missing required env var `DOWNLOADER_SERVICE_DNS`");
        let (host, port) = parse_host(&raw);

        Self {
            host: host.into_boxed_str(),
            port,
        }
    }

    #[must_use]
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Resolves the headless downloader service DNS into active node socket addresses.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if DNS resolution fails.
    pub async fn resolve_nodes(&self) -> io::Result<Vec<SocketAddr>> {
        Ok(tokio::net::lookup_host(self.authority())
            .await?
            .collect::<HashSet<_>>()
            .into_iter()
            .collect())
    }
}

#[derive(Clone)]
pub(crate) struct NodeClient {
    ca_cert_pem: Box<[u8]>,
    cert_pem: Box<[u8]>,
    key_pem: Box<[u8]>,
    server_name: Box<str>,
}

impl NodeClient {
    pub(crate) fn load(cfg: &DownloaderTlsConfig, server_name: &str) -> Self {
        let ca_cert_pem = fs::read(&*cfg.ca_cert).unwrap_or_else(|_| panic!("Failed to read downloader CA certificate {}", cfg.ca_cert));
        let cert_pem = fs::read(&*cfg.cert).unwrap_or_else(|_| panic!("Failed to read client certificate {}", cfg.cert));
        let key_pem = fs::read(&*cfg.key).unwrap_or_else(|_| panic!("Failed to read client key {}", cfg.key));

        Self {
            ca_cert_pem: ca_cert_pem.into(),
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
            server_name: server_name.into(),
        }
    }

    pub(crate) fn build_channel(&self, address: &str) -> Result<Channel, tonic::transport::Error> {
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
