use std::sync::Arc;

use telers::errors::HandlerError;

use crate::{
    config::Config,
    entities::ChatConfig,
    interactors::Interactor,
    locale::Locale,
    services::{
        help,
        messenger::{MessengerPort, SendTextRequest, TextFormat},
    },
    utils::ErrorFormatter,
};
use tracing::error;

pub struct Start<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
}

impl<Messenger> Start<Messenger> {
    #[must_use]
    pub const fn new(cfg: Arc<Config>, error_formatter: Arc<ErrorFormatter>, messenger: Arc<Messenger>) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
        }
    }
}

pub struct StartInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub chat_cfg: Option<&'a ChatConfig>,
}

impl<Messenger> Interactor<StartInput<'_>> for &Start<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: StartInput<'_>) -> Result<Self::Output, Self::Err> {
        let username = match self.messenger.username().await {
            Ok(username) => username,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get messenger username error");
                return Ok(());
            }
        };

        let locale = input.chat_cfg.map_or(Locale::En, ChatConfig::locale);
        let max_file_size_in_mb = self.cfg.yt_dlp.max_file_size / 1000 / 1000;
        let text = help::full(locale, &username, max_file_size_in_mb, &self.cfg.bot.src_url);

        if let Err(err) = self
            .messenger
            .send_text(SendTextRequest {
                chat_id: input.chat_id,
                text: &text,
                reply_to_message_id: input.reply_to_message_id,
                format: Some(TextFormat::Html),
                disable_link_preview: true,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }

        Ok(())
    }
}
