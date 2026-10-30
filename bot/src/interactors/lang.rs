use std::sync::Arc;

use rust_i18n::t;
use telers::errors::HandlerError;
use tracing::error;

use crate::{
    entities::{ChatConfig, ChatConfigUpdate},
    handlers_utils::progress,
    interactors::Interactor,
    locale::Locale,
    services::{
        chat,
        messenger::{EphemeralMenu, Keyboard, MessengerPort, SendMenuRequest, TextFormat},
    },
    utils::ErrorFormatter,
};

pub struct Lang<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    update_chat_cfg: Arc<chat::UpdateChatConfig>,
}

impl<Messenger> Lang<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        update_chat_cfg: Arc<chat::UpdateChatConfig>,
    ) -> Self {
        Self {
            error_formatter,
            messenger,
            update_chat_cfg,
        }
    }
}

pub struct LangInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub ephemeral: Option<EphemeralMenu>,
    pub chat_cfg: &'a ChatConfig,
    pub argument: Option<&'a str>,
}

impl<Messenger> Interactor<LangInput<'_>> for &Lang<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: LangInput<'_>) -> Result<Self::Output, Self::Err> {
        let current = input.chat_cfg.locale();
        let arg = input.argument.map(str::trim).filter(|s| !s.is_empty());

        let target = match arg {
            None => current.toggle(),
            Some(val) => {
                if let Some(locale) = Locale::parse(val) {
                    locale
                } else {
                    let text = t!("lang.unknown", locale = current.as_str(), lang = val);
                    self.send_text(input, &text).await;
                    return Ok(());
                }
            }
        };

        let text = match self
            .update_chat_cfg
            .execute(chat::UpdateChatConfigInput {
                dto: ChatConfigUpdate::new(input.chat_cfg.tg_id).with_language(target.as_str().to_owned()),
            })
            .await
        {
            Ok(_) => {
                let display_name = t!("lang.name", locale = target.as_str());
                t!("lang.changed", locale = target.as_str(), lang = display_name).into_owned()
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Update error");
                t!("lang.error", locale = current.as_str()).into_owned()
            }
        };

        self.send_text(input, &text).await;

        Ok(())
    }
}

impl<Messenger> Lang<Messenger>
where
    Messenger: MessengerPort,
{
    async fn send_text(&self, input: LangInput<'_>, text: &str) {
        let result = if let Some(ephemeral) = input.ephemeral {
            self.messenger
                .send_menu(SendMenuRequest {
                    chat_id: input.chat_id,
                    ephemeral: Some(ephemeral),
                    text,
                    keyboard: &Keyboard { rows: Vec::new() },
                })
                .await
        } else {
            progress::new(
                self.messenger.as_ref(),
                text,
                input.chat_id,
                input.reply_to_message_id,
                Some(TextFormat::Html),
            )
            .await
            .map(|_| ())
        };
        if let Err(err) = result {
            error!(err = %self.error_formatter.format(&err), "Send error");
        }
    }
}
