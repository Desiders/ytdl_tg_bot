use std::sync::Arc;

use reqwest::Client;
use telers::{client::telegram::APIServer, methods::GetFile, Bot};
use tracing::{instrument, warn};

use crate::config::{BotConfig, TelegramBotApiConfig};

#[derive(Debug, thiserror::Error)]
pub enum FileDownloadErrorKind {
    #[error("Telegram getFile error: {0}")]
    GetFile(String),
    #[error("Telegram file has no path")]
    MissingPath,
    #[error("File download request failed: {0}")]
    Http(String),
}

pub struct TelegramFileDownloader {
    bot: Arc<Bot>,
    client: Arc<Client>,
    api_server: Arc<APIServer>,
    token: Box<str>,
    file_server_url: Option<Box<str>>,
    work_dir: Option<Box<str>>,
}

impl TelegramFileDownloader {
    #[must_use]
    pub fn new(bot: Arc<Bot>, client: Arc<Client>, api_server: Arc<APIServer>, cfg: &TelegramBotApiConfig, bot_cfg: &BotConfig) -> Self {
        Self {
            bot,
            client,
            api_server,
            token: bot_cfg.token.clone(),
            file_server_url: cfg.file_server_url.clone(),
            work_dir: cfg.work_dir.clone(),
        }
    }

    #[instrument(skip_all, fields(file_id = file_id))]
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>, FileDownloadErrorKind> {
        let file = self
            .bot
            .send(GetFile::new(file_id))
            .await
            .map_err(|err| FileDownloadErrorKind::GetFile(err.to_string()))?;
        let path = file.file_path.ok_or(FileDownloadErrorKind::MissingPath)?;

        let file_server_url = match (&self.work_dir, &self.file_server_url) {
            (Some(work_dir), Some(file_server_url)) => path
                .strip_prefix(&**work_dir)
                .map(|relative| format!("{}{}", file_server_url.trim_end_matches('/'), relative)),
            _ => None,
        };

        let (url, deletable) = match file_server_url {
            Some(url) => (url, true),
            None => (String::from(self.api_server.file_url(&self.token, &path)), false),
        };

        let bytes = self
            .client
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|err| FileDownloadErrorKind::Http(err.to_string()))?
            .bytes()
            .await
            .map_err(|err| FileDownloadErrorKind::Http(err.to_string()))?;

        if deletable {
            if let Err(err) = self.client.delete(&url).send().await.and_then(reqwest::Response::error_for_status) {
                warn!(url, error = %err, "Failed to delete downloaded file after reading");
            }
        }

        Ok(bytes.to_vec())
    }
}
