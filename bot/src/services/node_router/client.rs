use anyhow::{Context, Result};
use std::fs;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};

use crate::config::{DownloadNodeConfig, DownloadTlsConfig};

pub(super) struct NodeClient {
    ca_cert_pem: Box<[u8]>,
    cert_pem: Box<[u8]>,
    key_pem: Box<[u8]>,
}

impl NodeClient {
    pub(super) fn load(config: &DownloadTlsConfig) -> Result<Self> {
        let ca_cert_pem =
            fs::read(&*config.ca_cert_path).with_context(|| format!("Failed to read downloader CA certificate {}", config.ca_cert_path))?;
        let cert_pem = fs::read(&*config.cert_path).with_context(|| format!("Failed to read bot certificate {}", config.cert_path))?;
        let key_pem = fs::read(&*config.key_path).with_context(|| format!("Failed to read bot key {}", config.key_path))?;

        Ok(Self {
            ca_cert_pem: ca_cert_pem.into(),
            cert_pem: cert_pem.into(),
            key_pem: key_pem.into(),
        })
    }

    pub(super) fn build_channel(&self, DownloadNodeConfig { address, .. }: &DownloadNodeConfig) -> Result<Channel> {
        let tls_cfg = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(self.ca_cert_pem.as_ref()))
            .identity(Identity::from_pem(self.cert_pem.as_ref(), self.key_pem.as_ref()));

        let endpoint = Endpoint::from_shared(address.to_string())
            .with_context(|| format!("Invalid node address {address}"))?
            .tls_config(tls_cfg)
            .with_context(|| format!("Invalid TLS configuration for node {address}"))?;
        Ok(endpoint.connect_lazy())
    }
}
