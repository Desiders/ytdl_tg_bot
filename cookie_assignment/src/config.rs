#![allow(clippy::module_name_repetitions)]

use serde::Deserialize;
use std::{
    env::{self, VarError},
    fs, io,
    path::Path,
};
use thiserror::Error;

#[derive(Deserialize, Clone, Debug)]
pub struct LoggingConfig {
    pub dirs: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DownloadTlsConfig {
    pub ca_cert_path: Box<str>,
    pub cert_path: Box<str>,
    pub key_path: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DownloadConfig {
    pub token: Box<str>,
    pub cookie_manager_token: Box<str>,
    pub tls: DownloadTlsConfig,
}

impl From<DownloadTlsConfig> for downloader_client::DownloaderTlsConfig {
    fn from(value: DownloadTlsConfig) -> Self {
        Self {
            ca_cert_path: value.ca_cert_path,
            cert_path: value.cert_path,
            key_path: value.key_path,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct SyncConfig {
    pub interval: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub logging: LoggingConfig,
    pub download: DownloadConfig,
    pub sync: SyncConfig,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error(transparent)]
    IO(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

#[must_use]
pub fn get_path() -> Box<str> {
    let path = match env::var("COOKIE_ASSIGNMENT_CONFIG_PATH") {
        Ok(val) => val,
        Err(VarError::NotPresent) => String::from("configs/cookie_assignment.toml"),
        Err(VarError::NotUnicode(_)) => {
            panic!("`COOKIE_ASSIGNMENT_CONFIG_PATH` env variable is not a valid UTF-8 string!");
        }
    };

    path.into_boxed_str()
}

#[allow(clippy::missing_errors_doc)]
pub fn parse_from_fs(path: impl AsRef<Path>) -> Result<Config, ParseError> {
    let raw = fs::read_to_string(path)?;
    let cfg = toml::from_str(&raw)?;
    Ok(cfg)
}
