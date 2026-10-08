use froodi::async_impl::Container;
use telers::{
    enums,
    errors::{EventErrorKind, MiddlewareError},
    event::EventReturn,
    middlewares::outer::{Middleware, MiddlewareResponse},
    types::Chat::Private,
    Request,
};
use tracing::{error, instrument, warn};

use crate::{
    entities::{Chat, ChatConfig, OwnChatConfig},
    interactors::Interactor as _,
    locale::Locale,
    services::chat,
    value_objects::ChatType,
};

#[derive(Clone)]
pub struct CreateChatMiddleware;

impl Middleware for CreateChatMiddleware {
    #[instrument(skip_all)]
    async fn call(&mut self, mut request: Request) -> Result<MiddlewareResponse, EventErrorKind> {
        // Guest chat IDs live in a separate namespace. Do not look up or persist their settings.
        if request.update.guest_message().is_some() {
            return Ok((request, EventReturn::Finish));
        }
        let (chat_id, cmd_random_enabled, username, chat_type) = match (request.update.chat(), request.update.from()) {
            (Some(chat), _) => {
                let chat_type = match ChatType::try_from(enums::ChatType::from(chat)) {
                    Ok(chat_type) => chat_type,
                    Err(err) => {
                        warn!(%err, chat_id = chat.id(), "Skipping update");
                        return Ok((request, EventReturn::Cancel));
                    }
                };
                (chat.id(), matches!(chat, Private(_)), chat.username(), chat_type)
            }
            (None, Some(from)) => (from.id, false, from.username.as_deref(), ChatType::Private),
            _ => return Ok((request, EventReturn::Finish)),
        };
        let Some(container) = request.extensions.get::<Container>() else {
            return Ok((request, EventReturn::Finish));
        };

        let language_code = request.update.from().and_then(|user| user.language_code.as_deref());
        let locale = Locale::from_code(language_code);

        let db_chat = Chat::new(chat_id, username.map(ToOwned::to_owned), chat_type);
        let db_chat_config = ChatConfig::new(chat_id, cmd_random_enabled, locale.as_str().to_owned());

        let save_chat = container.get::<chat::SaveChat>().await.unwrap();
        let get_chat_config = container.get::<chat::GetChatConfig>().await.unwrap();

        match save_chat
            .execute(chat::SaveChatInput {
                chat: db_chat,
                chat_config: db_chat_config,
            })
            .await
        {
            Ok((_, chat_config, chat_config_exclude_domains)) => {
                let own_chat_config = match chat_type {
                    ChatType::Private => Some(chat_config.clone()),
                    _ => match request.update.from() {
                        Some(from) => match get_chat_config.execute(chat::GetChatConfigInput { tg_id: from.id }).await {
                            Ok(chat_config) => chat_config,
                            Err(err) => return Err(MiddlewareError::new(err).into()),
                        },
                        None => None,
                    },
                };
                request.extensions.insert(chat_config);
                request.extensions.insert(OwnChatConfig(own_chat_config));
                request.extensions.insert(chat_config_exclude_domains);
            }
            Err(err) => error!(%err, "Save chat error"),
        }

        Ok((request, EventReturn::Finish))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn guest_bypasses_chat_registration_even_for_unknown_chat_types() {
        let update = serde_json::from_value(serde_json::json!({
            "update_id": 1,
            "guest_message": {
                "message_id": 1, "date": 1,
                "chat": {"id": 7, "type": "synthetic_unknown"},
                "from": {"id": 8, "is_bot": false, "first_name": "Synthetic", "language_code": "ru"},
                "guest_query_id": "synthetic-query", "text": "@synthetic_bot"
            }
        }))
        .unwrap();
        let request = Request {
            bot: telers::Bot::new("123456:synthetic-token"),
            update: Arc::new(update),
            context: telers::Context::default(),
            extensions: telers::Extensions::default(),
        };
        // No database/container is installed. Guest processing must not need either.
        let (request, action) = CreateChatMiddleware.call(request).await.unwrap();
        assert!(matches!(action, EventReturn::Finish));
        assert!(request.extensions.get::<ChatConfig>().is_none());
        assert!(request.extensions.get::<OwnChatConfig>().is_none());
    }
}
