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
    pub video_download: u64,
    pub audio_download: u64,
    pub send_by_fs: f32,
    pub send_by_id: f32,
}

impl Default for TimeoutsConfig {
    fn default() -> Self {
        Self {
            video_download: 420,
            audio_download: 420,
            send_by_fs: 210.0,
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
    pub max_file_size: u32,
    pub executable_path: Box<str>,
    pub cookies_path: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DatabaseConfig {
    pub host: Box<str>,
    pub port: i16,
    pub user: Box<str>,
    pub password: Box<str>,
    pub database: Box<str>,
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
pub struct YtPotProviderConfig {
    pub url: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct TelegramBotApiConfig {
    pub url: Box<str>,
    // pub api_id: Box<str>,
    // pub api_hash: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ReplaceDomainsConfig {
    pub from: Box<str>,
    pub to: Box<str>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub bot: BotConfig,
    pub chat: ChatConfig,
    pub logging: LoggingConfig,
    pub database: DatabaseConfig,
    pub yt_dlp: YtDlpConfig,
    pub yt_toolkit: YtToolkitConfig,
    pub yt_pot_provider: YtPotProviderConfig,
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
    pub replace_domains: Vec<ReplaceDomainsConfig>,
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
        Err(VarError::NotPresent) => String::from("config.toml"),
        Err(VarError::NotUnicode(_)) => {
            panic!("`CONFIG_PATH` env variable is not a valid UTF-8 string!");
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
