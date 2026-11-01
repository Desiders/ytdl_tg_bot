use std::sync::Arc;

use rust_i18n::t;
use telers::{errors::HandlerError, utils::text::html_quote};
use tracing::error;

use crate::{
    entities::ChatConfig,
    interactors::{
        menu::{Fsm, EPHEMERAL_MESSAGE_ID},
        Interactor,
    },
    services::messenger::{
        AnswerCallbackRequest, EditMenuRequest, EphemeralMessage, Keyboard, MenuMessage, MessengerPort, SendTextRequest,
    },
    utils::ErrorFormatter,
    value_objects::menu::MenuState,
};

pub struct CheckMenuAccess<Messenger> {
    messenger: Arc<Messenger>,
    error_formatter: Arc<ErrorFormatter>,
}

impl<Messenger> CheckMenuAccess<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>, error_formatter: Arc<ErrorFormatter>) -> Self {
        Self {
            messenger,
            error_formatter,
        }
    }
}

pub struct CheckMenuAccessInput<'a> {
    pub chat_id: i64,
    pub user_id: Option<i64>,
    pub menu_owner_id: Option<i64>,
    pub callback_id: Option<&'a str>,
    pub ephemeral_message_id: Option<i64>,
    pub require_admin: bool,
    pub chat_cfg: &'a ChatConfig,
    pub fsm: &'a Fsm,
}

impl<Messenger> Interactor<CheckMenuAccessInput<'_>> for &CheckMenuAccess<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = bool;
    type Err = HandlerError;

    async fn execute(self, input: CheckMenuAccessInput<'_>) -> Result<Self::Output, Self::Err> {
        let Some(user_id) = input.user_id else {
            return Ok(false);
        };
        if input.chat_id == user_id {
            return Ok(true);
        }

        let locale = input.chat_cfg.locale();
        if input.callback_id.is_some() && (input.ephemeral_message_id.is_none() || input.menu_owner_id != Some(user_id)) {
            self.deny(&input, &t!("menu.not_owner", locale = locale.as_str()), user_id).await;
            return Ok(false);
        }
        if !input.require_admin {
            return Ok(true);
        }

        let notice = match self.messenger.is_chat_administrator(input.chat_id, user_id).await {
            Ok(true) => return Ok(true),
            Ok(false) => t!("menu.admin_only", locale = locale.as_str()),
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Check menu administrator error");
                t!("menu.admin_check_error", locale = locale.as_str())
            }
        };
        self.deny(&input, &notice, user_id).await;
        if input.callback_id.is_none() && input.fsm.get_state().await.ok().flatten().as_deref() == Some(MenuState::DomainInput.as_ref()) {
            if let Err(err) = input.fsm.finish().await {
                error!(%err, "Menu state error");
            }
        }
        Ok(false)
    }
}

impl<Messenger> CheckMenuAccess<Messenger>
where
    Messenger: MessengerPort,
{
    async fn deny(&self, input: &CheckMenuAccessInput<'_>, text: &str, user_id: i64) {
        let result = if let Some(callback_id) = input.callback_id {
            self.messenger
                .answer_callback(AnswerCallbackRequest {
                    callback_id,
                    text: Some(text),
                })
                .await
        } else {
            let message_id = match input.fsm.get_value::<_, i64>(EPHEMERAL_MESSAGE_ID).await {
                Ok(message_id) => message_id,
                Err(err) => {
                    error!(%err, "Menu state error");
                    None
                }
            };
            if let Some(ephemeral_message_id) = message_id {
                self.messenger
                    .edit_menu(EditMenuRequest {
                        target: MenuMessage::EphemeralMessage {
                            chat_id: input.chat_id,
                            receiver_user_id: user_id,
                            ephemeral_message_id,
                        },
                        text: &html_quote(text),
                        keyboard: &Keyboard { rows: Vec::new() },
                    })
                    .await
            } else {
                self.messenger
                    .send_text(SendTextRequest {
                        chat_id: input.chat_id,
                        text,
                        reply_to_message_id: None,
                        ephemeral: Some(EphemeralMessage {
                            receiver_user_id: user_id,
                            reply_to_ephemeral_message_id: input.ephemeral_message_id,
                        }),
                        format: None,
                        disable_link_preview: true,
                    })
                    .await
                    .map(|_| ())
            }
        };
        if let Err(err) = result {
            error!(err = %self.error_formatter.format(&err), "Send menu access notice error");
        }
    }
}
