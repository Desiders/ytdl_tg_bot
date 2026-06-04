#![allow(clippy::module_name_repetitions)]

use serde::Deserialize;
use std::{
    env::{self, VarError},
    fs, io,
    path::Path,
};
use thiserror::Error;

#[derive(Deserialize, Clone, Debug)]
pub struct BotConfig {
    pub token: Box<str>,
    pub src_url: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ChatConfig {
    pub receiver_chat_id: i64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TimeoutsConfig {
    pub send_by_upload: f32,
    pub send_by_id: f32,
}

impl Default for TimeoutsConfig {
    fn default() -> Self {
        Self {
            send_by_upload: 210.0,
            send_by_id: 360.0,
        }
    }
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct BlacklistedConfig {
    #[serde(default)]
    pub domains: Vec<String>,
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct DomainsWithReactionsConfig {
    #[serde(default)]
    pub domains: Vec<String>,
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct RandomCmdConfig {
    #[serde(default)]
    pub domains: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LoggingConfig {
    pub dirs: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct YtDlpConfig {
    pub max_file_size: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DatabaseConfig {
    pub host: Box<str>,
    pub port: i16,
    pub user: Box<str>,
    pub password: Box<str>,
    pub database: Box<str>,
    #[serde(default = "default_db_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_db_acquire_timeout_secs")]
    pub acquire_timeout_secs: u64,
    #[serde(default = "default_db_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
}

const fn default_db_max_connections() -> u32 {
    20
}

const fn default_db_acquire_timeout_secs() -> u64 {
    30
}

const fn default_db_connect_timeout_secs() -> u64 {
    10
}

#[derive(Default, Deserialize, Clone, Debug)]
pub struct TrackingParamsConfig {
    #[serde(default)]
    pub params: Vec<Box<str>>,
}

impl DatabaseConfig {
    pub fn get_postgres_url(&self) -> String {
        format!(
            "postgres://{user}:{password}@{host}:{port}/{database}",
            user = self.user,
            password = self.password,
            host = self.host,
            port = self.port,
            database = self.database,
        )
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct YtToolkitConfig {
    pub url: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TelegramBotApiConfig {
    pub url: Box<str>,
    // pub api_id: Box<str>,
    // pub api_hash: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DownloaderTlsConfig {
    #[serde(rename = "ca_cert_path")]
    pub ca_cert: Box<str>,
    #[serde(rename = "cert_path")]
    pub cert: Box<str>,
    #[serde(rename = "key_path")]
    pub key: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DownloadConfig {
    #[serde(default)]
    pub capabilities_refresh_interval: u64,
    pub node_token: Box<str>,
    pub tls: DownloaderTlsConfig,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RedisConfig {
    pub host: Box<str>,
    pub port: u16,
    #[serde(default)]
    pub password: Option<Box<str>>,
    #[serde(default)]
    pub db: u8,
    #[serde(default)]
    pub queue: QueueConfig,
}

impl RedisConfig {
    #[must_use]
    pub fn get_url(&self) -> String {
        match &self.password {
            Some(password) => format!(
                "redis://:{password}@{host}:{port}/{db}",
                host = self.host,
                port = self.port,
                db = self.db
            ),
            None => format!("redis://{host}:{port}/{db}", host = self.host, port = self.port, db = self.db),
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct QueueConfig {
    #[serde(default = "default_queue_workers")]
    pub workers: usize,
    #[serde(default = "default_queue_stream_key")]
    pub stream_key: Box<str>,
    #[serde(default = "default_queue_group")]
    pub group: Box<str>,
    #[serde(default = "default_queue_dead_letter_key")]
    pub dead_letter_key: Box<str>,
    #[serde(default = "default_queue_max_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_queue_block_ms")]
    pub block_ms: u64,
    #[serde(default = "default_queue_claim_min_idle_ms")]
    pub claim_min_idle_ms: u64,
    #[serde(default = "default_queue_dedup_ttl_secs")]
    pub dedup_ttl_secs: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            workers: default_queue_workers(),
            stream_key: default_queue_stream_key(),
            group: default_queue_group(),
            dead_letter_key: default_queue_dead_letter_key(),
            max_attempts: default_queue_max_attempts(),
            block_ms: default_queue_block_ms(),
            claim_min_idle_ms: default_queue_claim_min_idle_ms(),
            dedup_ttl_secs: default_queue_dedup_ttl_secs(),
        }
    }
}

const fn default_queue_workers() -> usize {
    4
}

fn default_queue_stream_key() -> Box<str> {
    "ytdl:downloads".into()
}

fn default_queue_group() -> Box<str> {
    "workers".into()
}

fn default_queue_dead_letter_key() -> Box<str> {
    "ytdl:downloads:dead".into()
}

const fn default_queue_max_attempts() -> u32 {
    3
}

const fn default_queue_block_ms() -> u64 {
    5_000
}

const fn default_queue_claim_min_idle_ms() -> u64 {
    60_000
}

const fn default_queue_dedup_ttl_secs() -> u64 {
    3_600
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub bot: BotConfig,
    pub chat: ChatConfig,
    pub logging: LoggingConfig,
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub yt_dlp: YtDlpConfig,
    pub yt_toolkit: YtToolkitConfig,
    pub download: DownloadConfig,
    pub telegram_bot_api: TelegramBotApiConfig,
    #[serde(default)]
    pub timeouts: TimeoutsConfig,
    #[serde(default)]
    pub blacklisted: BlacklistedConfig,
    #[serde(default)]
    pub domains_with_reactions: DomainsWithReactionsConfig,
    #[serde(default)]
    pub random_cmd: RandomCmdConfig,
    #[serde(default)]
    pub tracking_params: TrackingParamsConfig,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error(transparent)]
    IO(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
}

/// # Panics
///
/// Panics if the `CONFIG_PATH` environment variable is not valid UTF-8.
#[must_use]
pub fn get_path() -> Box<str> {
    let path = match env::var("CONFIG_PATH") {
        Ok(val) => val,
        Err(VarError::NotPresent) => String::from("configs/config.toml"),
        Err(VarError::NotUnicode(_)) => {
            panic!("`CONFIG_PATH` env variable is not a valid UTF-8 string!");
        }
    };

    path.into_boxed_str()
}

/// Loads bot configuration from a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be read or the TOML cannot be parsed.
pub fn parse_from_fs(path: impl AsRef<Path>) -> Result<Config, ParseError> {
    let raw = fs::read_to_string(path)?;
    let cfg = toml::from_str(&raw)?;
    Ok(cfg)
}

impl From<DownloaderTlsConfig> for downloader_client::DownloaderTlsConfig {
    fn from(value: DownloaderTlsConfig) -> Self {
        Self {
            ca_cert: value.ca_cert,
            cert: value.cert,
            key: value.key,
        }
    }
}

impl From<DownloadConfig> for downloader_client::DownloaderClusterConfig {
    fn from(value: DownloadConfig) -> Self {
        Self {
            node_token: value.node_token,
            tls: value.tls.into(),
        }
    }
}
