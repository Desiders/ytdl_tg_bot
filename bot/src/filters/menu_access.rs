use std::sync::Arc;

use froodi::async_impl::Container;
use rust_i18n::t;
use telers::{
    callback_data::CallbackData as _,
    errors::{EventErrorKind, HandlerError},
    event::{telegram::HandlerResponse, EventReturn},
    filters::Filter,
    middlewares::inner::{Middleware, Next},
    types::{CallbackQuery, Chat as TelegramChat},
    utils::text::html_quote,
    FilterResult, Request,
};
use tracing::error;

use crate::{
    entities::{Chat, ChatConfig, OwnChatConfig},
    interactors::{
        menu::{Fsm, EPHEMERAL_MESSAGE_ID},
        Interactor as _,
    },
    locale::Locale,
    services::{
        chat,
        messenger::{AnswerCallbackRequest, EditMenuRequest, EphemeralMenu, Keyboard, MenuMessage, MessengerPort, SendMenuRequest},
    },
    utils::ErrorFormatter,
    value_objects::{
        menu::{DeleteDomain, MenuState, OpenScreen, Screen},
        ChatType,
    },
};

pub struct MenuAccess<Messenger> {
    messenger: Arc<Messenger>,
    error_formatter: Arc<ErrorFormatter>,
    require_admin: bool,
}

impl<Messenger> Clone for MenuAccess<Messenger> {
    fn clone(&self) -> Self {
        Self {
            messenger: self.messenger.clone(),
            error_formatter: self.error_formatter.clone(),
            require_admin: self.require_admin,
        }
    }
}

impl<Messenger> MenuAccess<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>, error_formatter: Arc<ErrorFormatter>) -> Self {
        Self {
            messenger,
            error_formatter,
            require_admin: false,
        }
    }

    #[must_use]
    pub const fn require_admin(mut self) -> Self {
        self.require_admin = true;
        self
    }
}

impl<Messenger> Filter for MenuAccess<Messenger>
where
    Messenger: MessengerPort + 'static,
{
    type Error = HandlerError;

    async fn check(&mut self, request: &mut Request) -> FilterResult<Self::Error> {
        self.authorize(request).await
    }
}

impl<Messenger> Middleware for MenuAccess<Messenger>
where
    Messenger: MessengerPort + 'static,
{
    async fn call(&mut self, mut request: Request, next: Next) -> Result<HandlerResponse, EventErrorKind> {
        if self.authorize(&mut request).await.map_err(EventErrorKind::Handler)? {
            next(request).await
        } else {
            Ok(HandlerResponse {
                request,
                result: Ok(EventReturn::Finish),
            })
        }
    }
}

impl<Messenger> MenuAccess<Messenger>
where
    Messenger: MessengerPort,
{
    async fn authorize(&self, request: &mut Request) -> FilterResult<HandlerError> {
        let Some(chat) = menu_chat(request) else {
            return Ok(false);
        };
        if matches!(chat, TelegramChat::Private(_)) {
            return Ok(true);
        }
        if !matches!(chat, TelegramChat::Group(_) | TelegramChat::Supergroup(_)) {
            return Ok(false);
        }
        let Some(user) = request.update.from() else {
            return Ok(false);
        };
        if user.is_bot || request.update.sender_chat().is_some() {
            return Ok(false);
        }
        let locale = request
            .extensions
            .get::<OwnChatConfig>()
            .and_then(|config| config.0.as_ref())
            .map_or_else(|| Locale::from_code(user.language_code.as_deref()), ChatConfig::locale);

        if let Some(query) = request.update.callback_query() {
            if !is_menu_owner(query) {
                self.deny(request, &t!("menu.not_owner", locale = locale.as_str())).await;
                return Ok(false);
            }
        }

        let requires_admin = self.require_admin
            || request
                .update
                .callback_query()
                .and_then(|query| query.data.as_deref())
                .is_some_and(is_domain_action);
        if requires_admin {
            let notice = match self.messenger.is_chat_administrator(chat.id(), user.id).await {
                Ok(true) => None,
                Ok(false) => Some(t!("menu.admin_only", locale = locale.as_str())),
                Err(err) => {
                    error!(err = %self.error_formatter.format(&err), "Check menu administrator error");
                    Some(t!("menu.admin_check_error", locale = locale.as_str()))
                }
            };
            if let Some(notice) = notice {
                self.deny(request, &notice).await;
                if self.require_admin && request.update.message().is_some() {
                    cancel_domain_input(request).await;
                }
                return Ok(false);
            }
        }

        if request
            .extensions
            .get::<OwnChatConfig>()
            .and_then(|config| config.0.as_ref())
            .is_none()
        {
            let container = request
                .extensions
                .get::<Container>()
                .ok_or_else(|| HandlerError::from_display("Missing request container"))?;
            let save_chat = container.get::<chat::SaveChat>().await.map_err(HandlerError::new)?;
            let (_, config, _) = save_chat
                .as_ref()
                .execute(chat::SaveChatInput {
                    chat: Chat::new(user.id, user.username.as_deref().map(ToOwned::to_owned), ChatType::Private),
                    chat_config: ChatConfig::new(user.id, true, locale.as_str().to_owned()),
                })
                .await
                .map_err(HandlerError::new)?;
            request.extensions.insert(OwnChatConfig(Some(config)));
        }
        Ok(true)
    }

    async fn deny(&self, request: &Request, text: &str) {
        let result = if let Some(query) = request.update.callback_query() {
            self.messenger
                .answer_callback(AnswerCallbackRequest {
                    callback_id: &query.id,
                    text: Some(text),
                })
                .await
        } else if let Some(message) = request.update.message() {
            let Some(user) = message.from() else {
                return;
            };
            let ephemeral_message_id = match request.context.get::<Fsm>("fsm_context") {
                Some(fsm) => match fsm.get_value::<_, i64>(EPHEMERAL_MESSAGE_ID).await {
                    Ok(message_id) => message_id,
                    Err(err) => {
                        error!(%err, "Menu state error");
                        None
                    }
                },
                None => None,
            };
            let text = html_quote(text);
            let keyboard = Keyboard { rows: Vec::new() };
            if let Some(ephemeral_message_id) = ephemeral_message_id {
                self.messenger
                    .edit_menu(EditMenuRequest {
                        message: MenuMessage::Ephemeral {
                            chat_id: message.chat().id(),
                            receiver_user_id: user.id,
                            ephemeral_message_id,
                        },
                        text: &text,
                        keyboard: &keyboard,
                    })
                    .await
            } else {
                self.messenger
                    .send_menu(SendMenuRequest {
                        chat_id: message.chat().id(),
                        ephemeral: Some(EphemeralMenu {
                            receiver_user_id: user.id,
                            reply_to_ephemeral_message_id: message.ephemeral_message_id(),
                        }),
                        text: &text,
                        keyboard: &keyboard,
                    })
                    .await
            }
        } else {
            return;
        };
        if let Err(err) = result {
            error!(err = %self.error_formatter.format(&err), "Send menu access notice error");
        }
    }
}

fn menu_chat(request: &Request) -> Option<&TelegramChat> {
    // UserContext also resolves callback messages, which Update::chat does not cover.
    request.context.get::<TelegramChat>("event_chat").or_else(|| request.update.chat())
}

fn is_menu_owner(query: &CallbackQuery) -> bool {
    query.message.as_deref().is_some_and(|message| {
        message.ephemeral_message_id().is_some() && message.receiver_user().is_some_and(|receiver| receiver.id == query.from.id)
    })
}

fn is_domain_action(data: &str) -> bool {
    DeleteDomain::unpack(data).is_ok()
        || OpenScreen::unpack(data).is_ok_and(|data| {
            matches!(
                data.screen,
                Screen::Domains | Screen::DomainAdd | Screen::Domain { .. } | Screen::DomainDelete { .. }
            )
        })
}

async fn cancel_domain_input(request: &Request) {
    let Some(fsm) = request.context.get::<Fsm>("fsm_context") else {
        return;
    };
    if fsm.get_state().await.ok().flatten().as_deref() == Some(MenuState::DomainInput.as_ref()) {
        if let Err(err) = fsm.finish().await {
            error!(%err, "Menu state error");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_objects::menu::{DomainKey, SetLanguage};
    use telers::{
        event::bases::PropagateEventResult,
        middlewares::outer::{Middleware as OuterMiddleware, Next as OuterNext, UserContext},
        router::Response,
        serialization,
        types::{ChatGroup, Message, MessageText, User},
        Bot, Context, Extensions,
    };

    #[tokio::test]
    async fn callback_chats_are_resolved_by_sdk_user_context() {
        for (chat_type, chat_id) in [("private", 2), ("group", -10)] {
            let chat = serde_json::json!({"id": chat_id, "type": chat_type});
            let update = serde_json::json!({
                "update_id": 1,
                "callback_query": {
                    "id": "synthetic-callback", "chat_instance": "synthetic", "data": "open:settings",
                    "from": {"id": 2, "is_bot": false, "first_name": "Owner"},
                    "message": {
                        "message_id": 3, "date": 1, "chat": chat, "text": "Menu",
                        "ephemeral_message_id": (chat_type == "group").then_some(5),
                        "receiver_user": (chat_type == "group")
                            .then(|| serde_json::json!({"id":2, "is_bot":false, "first_name":"Owner"}))
                    }
                }
            });
            let request = Request {
                bot: Bot::new("1:synthetic"),
                update: Arc::new(serialization::from_str(&update.to_string()).unwrap()),
                context: Context::default(),
                extensions: Extensions::default(),
            };
            assert!(request.update.chat().is_none());
            assert!(menu_chat(&request).is_none());
            let next: OuterNext = Box::new(|request| Box::pin(async move { Ok(Response::new(request, PropagateEventResult::Unhandled)) }));
            let response = OuterMiddleware::call(&mut UserContext, request, next).await.unwrap();
            let resolved = menu_chat(&response.request).unwrap();
            assert_eq!(resolved.id(), chat_id);
            assert_eq!(matches!(resolved, TelegramChat::Private(_)), chat_type == "private");
            assert_eq!(response.request.update.from().unwrap().id, 2);
            assert_eq!(
                is_menu_owner(response.request.update.callback_query().unwrap()),
                chat_type == "group"
            );
        }
    }

    #[test]
    fn group_menu_owner_is_the_ephemeral_receiver() {
        let menu = MessageText::new(3, 1, ChatGroup::new(-1), "Menu")
            .ephemeral_message_id(5)
            .receiver_user(User::new(2, false, "Owner"));
        for (user_id, expected) in [(2, true), (4, false)] {
            let query = CallbackQuery::new("synthetic", User::new(user_id, false, "User"), "synthetic")
                .message(Message::Text(menu.clone()))
                .data(OpenScreen { screen: Screen::Main }.pack().unwrap());
            assert_eq!(is_menu_owner(&query), expected);
        }
        let public_menu = MessageText::new(3, 1, ChatGroup::new(-1), "Public menu")
            .reply_to_message(MessageText::new(1, 1, ChatGroup::new(-1), "/start").from(User::new(2, false, "Owner")));
        let query = CallbackQuery::new("synthetic", User::new(2, false, "Owner"), "synthetic").message(Message::Text(public_menu));
        assert!(!is_menu_owner(&query));
        for (ephemeral_message_id, receiver_user_id) in [(None, Some(2)), (Some(5), None), (Some(5), Some(4))] {
            let menu = MessageText::new(3, 1, ChatGroup::new(-1), "Menu")
                .ephemeral_message_id_option(ephemeral_message_id)
                .receiver_user_option(receiver_user_id.map(|id| User::new(id, false, "Receiver")));
            let query = CallbackQuery::new("synthetic", User::new(2, false, "Owner"), "synthetic").message(Message::Text(menu));
            assert!(!is_menu_owner(&query));
        }
    }

    #[test]
    fn every_domain_callback_requires_admin_permission() {
        let key = DomainKey::new("synthetic.example");
        for screen in [
            Screen::Domains,
            Screen::DomainAdd,
            Screen::Domain { key },
            Screen::DomainDelete { key },
        ] {
            assert!(is_domain_action(&OpenScreen { screen }.pack().unwrap()));
        }
        assert!(is_domain_action(&DeleteDomain { domain: key }.pack().unwrap()));
        assert!(!is_domain_action(&OpenScreen { screen: Screen::Settings }.pack().unwrap()));
        assert!(!is_domain_action(&SetLanguage { locale: Locale::Ru }.pack().unwrap()));
        assert!(!is_domain_action("open:invalid"));
    }
}
