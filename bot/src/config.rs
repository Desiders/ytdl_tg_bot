#![allow(clippy::module_name_repetitions)]

use serde::Deserialize;
use std::{
    env::{self, VarError},
    fs, io,
    path::Path,
    time::Duration,
};
use thiserror::Error;

const QUEUE_CLEANUP_MARGIN_SECS: u64 = 120;
pub(crate) const PROGRESS_REFRESH_INTERVAL_SECS: u64 = 60;
const DEFAULT_QUEUE_CLAIM_MIN_IDLE_MS: u64 =
    (proto::MEDIA_EXECUTION_LIMIT_SECS + PROGRESS_REFRESH_INTERVAL_SECS + QUEUE_CLEANUP_MARGIN_SECS) * 1_000;

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
    pub send_by_id: f32,
}

impl TimeoutsConfig {
    fn validate(&self) -> Result<(), ParseError> {
        if !self.send_by_id.is_finite() || self.send_by_id <= 0.0 || Duration::try_from_secs_f32(self.send_by_id).is_err() {
            return Err(ParseError::InvalidTimeout {
                field: "send_by_id",
                configured: self.send_by_id,
            });
        }
        Ok(())
    }
}

impl Default for TimeoutsConfig {
    fn default() -> Self {
        Self { send_by_id: 360.0 }
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

#[derive(Default, Deserialize, Clone, Debug)]
pub struct AudioFirstConfig {
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
    #[serde(default)]
    pub file_server_url: Option<Box<str>>,
    #[serde(default)]
    pub work_dir: Option<Box<str>>,
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
    pub user: Option<Box<str>>,
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
        let auth = match (self.user.as_deref(), self.password.as_deref()) {
            (None, None) => String::new(),
            (user, password) => format!("{}:{}@", user.unwrap_or(""), password.unwrap_or("")),
        };
        format!("redis://{auth}{host}:{port}/{db}", host = self.host, port = self.port, db = self.db)
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct QueueConfig {
    #[serde(default = "default_queue_stream_key")]
    pub stream_key: Box<str>,
    #[serde(default = "default_queue_group")]
    pub group: Box<str>,
    #[serde(default = "default_queue_block_ms")]
    pub block_ms: u64,
    #[serde(default = "default_queue_claim_min_idle_ms")]
    pub claim_min_idle_ms: u64,
    #[serde(default = "default_queue_dedup_ttl_secs")]
    pub dedup_ttl_secs: u64,
}

impl QueueConfig {
    fn validate(&self) -> Result<(), ParseError> {
        let minimum = proto::MEDIA_EXECUTION_LIMIT_SECS
            .saturating_add(QUEUE_CLEANUP_MARGIN_SECS + PROGRESS_REFRESH_INTERVAL_SECS)
            .saturating_mul(1_000);
        if self.claim_min_idle_ms < minimum {
            return Err(ParseError::ClaimMinIdleTooShort {
                configured: self.claim_min_idle_ms,
                minimum,
            });
        }
        Ok(())
    }
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            stream_key: default_queue_stream_key(),
            group: default_queue_group(),
            block_ms: default_queue_block_ms(),
            claim_min_idle_ms: default_queue_claim_min_idle_ms(),
            dedup_ttl_secs: default_queue_dedup_ttl_secs(),
        }
    }
}

fn default_queue_stream_key() -> Box<str> {
    "ytdl:downloads".into()
}

fn default_queue_group() -> Box<str> {
    "workers".into()
}

const fn default_queue_block_ms() -> u64 {
    5_000
}

const fn default_queue_claim_min_idle_ms() -> u64 {
    DEFAULT_QUEUE_CLAIM_MIN_IDLE_MS
}

const fn default_queue_dedup_ttl_secs() -> u64 {
    86_400
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
    pub audio_first: AudioFirstConfig,
    #[serde(default)]
    pub tracking_params: TrackingParamsConfig,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error(transparent)]
    IO(#[from] io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error(
        "`[redis.queue].claim_min_idle_ms` must be at least {minimum} ms to avoid discarding active work (configured: {configured} ms)"
    )]
    ClaimMinIdleTooShort { configured: u64, minimum: u64 },
    #[error("`[timeouts].{field}` must be a positive finite duration (configured: {configured} seconds)")]
    InvalidTimeout { field: &'static str, configured: f32 },
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
    let cfg: Config = toml::from_str(&raw)?;
    cfg.timeouts.validate()?;
    cfg.redis.queue.validate()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_cleanup_covers_media_limit_and_refresh_interval() {
        let mut queue = QueueConfig::default();
        assert!(queue.validate().is_ok());

        queue.claim_min_idle_ms = DEFAULT_QUEUE_CLAIM_MIN_IDLE_MS - 1;
        assert!(matches!(queue.validate(), Err(ParseError::ClaimMinIdleTooShort { .. })));
        queue.claim_min_idle_ms = DEFAULT_QUEUE_CLAIM_MIN_IDLE_MS;
        assert!(queue.validate().is_ok());
    }

    #[test]
    fn send_by_id_timeout_is_a_positive_finite_duration() {
        let defaults = TimeoutsConfig::default();
        assert_eq!(Duration::from_secs_f32(defaults.send_by_id), Duration::from_secs(360));
        assert!(defaults.validate().is_ok());

        let mut too_short = defaults;
        too_short.send_by_id = 0.0;
        assert!(matches!(too_short.validate(), Err(ParseError::InvalidTimeout { .. })));

        too_short.send_by_id = f32::NAN;
        assert!(matches!(too_short.validate(), Err(ParseError::InvalidTimeout { .. })));
    }
}
