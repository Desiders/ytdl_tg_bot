#![allow(clippy::module_name_repetitions)]

use serde::Deserialize;
use std::{
    env::{self, VarError},
    fs, io,
    path::Path,
};
use thiserror::Error;

#[derive(Deserialize, Clone, Debug)]
pub struct ServerConfig {
    pub address: Box<str>,
    pub max_concurrent: u32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct AuthConfig {
    pub tokens: Vec<Box<str>>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct YtDlpConfig {
    pub executable_path: Box<str>,
    pub cookies_path: Box<str>,
    pub max_file_size: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct YtPotProviderConfig {
    pub url: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LoggingConfig {
    pub dirs: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub server: ServerConfig,
    pub auth: AuthConfig,
    pub yt_dlp: YtDlpConfig,
    pub yt_pot_provider: YtPotProviderConfig,
    pub logging: LoggingConfig,
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
    let path = match env::var("NODE_CONFIG_PATH") {
        Ok(val) => val,
        Err(VarError::NotPresent) => String::from("node_config.toml"),
        Err(VarError::NotUnicode(_)) => {
            panic!("`NODE_CONFIG_PATH` env variable is not a valid UTF-8 string!");
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
