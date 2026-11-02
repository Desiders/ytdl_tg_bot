use std::sync::Arc;

use rust_i18n::t;
use telers::utils::text::html_quote;
use tracing::error;

use crate::{
    entities::ChatConfig,
    services::messenger::{AnswerCallbackRequest, EditMenuRequest, Keyboard, MenuMessage, MessengerPort, SendTextRequest},
    utils::ErrorFormatter,
    value_objects::ChatType,
};

pub struct MenuAccess<Messenger> {
    messenger: Arc<Messenger>,
    error_formatter: Arc<ErrorFormatter>,
}

impl<Messenger> MenuAccess<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>, error_formatter: Arc<ErrorFormatter>) -> Self {
        Self {
            messenger,
            error_formatter,
        }
    }
}

#[derive(Clone, Copy)]
pub struct MenuAccessInput<'a> {
    pub chat_id: i64,
    pub chat_type: ChatType,
    pub user_id: Option<i64>,
    pub is_bot: bool,
    pub sender_chat_id: Option<i64>,
    pub menu_owner_id: Option<i64>,
    pub callback_id: Option<&'a str>,
    pub ephemeral_message_id: Option<i64>,
}

impl<Messenger> MenuAccess<Messenger>
where
    Messenger: MessengerPort,
{
    pub async fn check(
        &self,
        input: &MenuAccessInput<'_>,
        chat_cfg: &ChatConfig,
        require_admin: bool,
        notice_target: Option<MenuMessage>,
    ) -> bool {
        if !matches!(input.chat_type, ChatType::Private | ChatType::Group | ChatType::Supergroup)
            || input.is_bot
            || input.sender_chat_id.is_some()
        {
            return false;
        }
        let Some(user_id) = input.user_id else {
            return false;
        };
        if input.chat_type == ChatType::Private {
            return true;
        }

        let locale = chat_cfg.locale();
        if input.callback_id.is_some() && (input.ephemeral_message_id.is_none() || input.menu_owner_id != Some(user_id)) {
            self.deny(input, &t!("menu.not_owner", locale = locale.as_str()), user_id, notice_target)
                .await;
            return false;
        }
        if !require_admin {
            return true;
        }

        let notice = match self.messenger.is_chat_administrator(input.chat_id, user_id).await {
            Ok(true) => return true,
            Ok(false) => t!("menu.admin_only", locale = locale.as_str()),
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Check menu administrator error");
                t!("menu.admin_check_error", locale = locale.as_str())
            }
        };
        self.deny(input, &notice, user_id, notice_target).await;
        false
    }

    async fn deny(&self, input: &MenuAccessInput<'_>, text: &str, user_id: i64, notice_target: Option<MenuMessage>) {
        let result = if let Some(callback_id) = input.callback_id {
            self.messenger
                .answer_callback(AnswerCallbackRequest {
                    callback_id,
                    text: Some(text),
                })
                .await
        } else if let Some(target @ MenuMessage::EphemeralMessage { .. }) = notice_target {
            self.messenger
                .edit_menu(EditMenuRequest {
                    target,
                    text: &html_quote(text),
                    keyboard: &Keyboard { rows: Vec::new() },
                })
                .await
        } else {
            self.messenger
                .send_text(SendTextRequest {
                    chat_id: user_id,
                    text,
                    reply_to_message_id: None,
                    ephemeral: None,
                    format: None,
                    disable_link_preview: true,
                })
                .await
                .map(|_| ())
        };
        if let Err(err) = result {
            error!(err = %self.error_formatter.format(&err), "Send menu access notice error");
        }
    }
}
