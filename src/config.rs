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
pub struct BlacklistedConfig {
    pub domains: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DomainsWithReactions {
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
pub struct Config {
    pub bot: BotConfig,
    pub chat: ChatConfig,
    pub blacklisted: BlacklistedConfig,
    pub logging: LoggingConfig,
    pub database: DatabaseConfig,
    pub yt_dlp: YtDlpConfig,
    pub yt_toolkit: YtToolkitConfig,
    pub yt_pot_provider: YtPotProviderConfig,
    pub telegram_bot_api: TelegramBotApiConfig,
    pub domains_with_reactions: DomainsWithReactions,
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
