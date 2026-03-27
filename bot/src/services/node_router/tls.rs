use anyhow::{Context, Result};
use std::{fs, sync::Arc};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};

use crate::config::{DownloadNodeConfig, DownloadTlsConfig};

pub(super) struct DownloadClientTls {
    ca_certificate_pem: Arc<[u8]>,
}

impl DownloadClientTls {
    pub(super) fn load(config: Option<&DownloadTlsConfig>) -> Result<Option<Self>> {
        let Some(config) = config else {
            return Ok(None);
        };

        let ca_certificate_pem =
            fs::read(&*config.ca_cert_path).with_context(|| format!("Failed to read downloader CA certificate {}", config.ca_cert_path))?;

        Ok(Some(Self {
            ca_certificate_pem: ca_certificate_pem.into(),
        }))
    }
}

pub(super) fn build_channel(config: &DownloadNodeConfig, tls: Option<&DownloadClientTls>) -> Result<Channel> {
    let address = &config.address;
    let mut endpoint = Endpoint::from_shared(config.address.to_string()).with_context(|| format!("Invalid node address {address}"))?;

    if address.starts_with("https://") {
        let tls = tls.context("HTTPS download node requires [download.tls] ca_cert_path")?;
        endpoint = endpoint
            .tls_config(ClientTlsConfig::new().ca_certificate(Certificate::from_pem(tls.ca_certificate_pem.as_ref())))
            .with_context(|| format!("Invalid TLS configuration for node {address}"))?;
    }

    Ok(endpoint.connect_lazy())
}
