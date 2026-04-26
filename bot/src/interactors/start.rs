use std::sync::Arc;

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_quote, html_text_link},
};

use crate::{
    config::Config,
    entities::ChatConfig,
    interactors::Interactor,
    locale::Locale,
    services::messenger::{MessengerPort, SendTextRequest, TextFormat},
    utils::ErrorFormatter,
};
use tracing::error;

pub struct Start<Messenger> {
    pub cfg: Arc<Config>,
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
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
        let source_label = t!("start.source_code_label", locale = locale.as_str()).into_owned();
        let source_code = html_text_link(source_label.as_str(), html_quote(&self.cfg.bot.src_url));

        let text = t!(
            "start.body",
            locale = locale.as_str(),
            username = username.as_str(),
            max_file_size_in_mb = max_file_size_in_mb,
            source_code = source_code,
        )
        .into_owned();

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
