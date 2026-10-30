use froodi::async_impl::Container;
use telers::{
    enums,
    errors::MiddlewareError,
    event::bases::PropagateEventResult,
    middlewares::outer::{Middleware, MiddlewareResult, Next},
    router::Response,
    types::Chat as TelegramChat,
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
    async fn call(&mut self, mut request: Request, next: Next) -> MiddlewareResult {
        // Guest chat IDs live in a separate namespace. Do not look up or persist their settings.
        if request.update.guest_message().is_some() {
            return next(request).await;
        }
        let event_chat = request.context.get::<TelegramChat>("event_chat").or_else(|| request.update.chat());
        let (chat_id, cmd_random_enabled, username, chat_type) = match (event_chat, request.update.from()) {
            (Some(chat), _) => {
                let chat_type = match ChatType::try_from(enums::ChatType::from(chat)) {
                    Ok(chat_type) => chat_type,
                    Err(err) => {
                        warn!(%err, chat_id = chat.id(), "Skipping update");
                        return Ok(Response::new(request, PropagateEventResult::Rejected));
                    }
                };
                (chat.id(), matches!(chat, TelegramChat::Private(_)), chat.username(), chat_type)
            }
            (None, Some(from)) => (from.id, false, from.username.as_deref(), ChatType::Private),
            _ => return next(request).await,
        };
        let Some(container) = request.extensions.get::<Container>() else {
            return next(request).await;
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
                            Err(err) => return Err((MiddlewareError::new(err).into(), request)),
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

        next(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, sync::Arc};

    use telers::{
        enums::UpdateType,
        event::{telegram::Handler, EventReturn},
        router::{PropagateEvent as _, Router},
        serialization, Bot, Context, Extensions,
    };

    use super::*;

    #[derive(Clone)]
    struct ReachedNext;

    fn request(kind: &str) -> Request {
        let query_id = if kind == "guest_message" {
            r#", "guest_query_id":"synthetic-query""#
        } else {
            ""
        };
        let update = format!(
            r#"{{"update_id":1,"{kind}":{{"message_id":2,"date":1,"chat":{{"id":3,"type":"synthetic_unknown"}},"text":"https://example.test/media"{query_id}}}}}"#
        );
        Request {
            bot: Bot::new("1:synthetic"),
            update: Arc::new(serialization::from_str(&update).unwrap()),
            context: Context::default(),
            extensions: Extensions::default(),
        }
    }

    #[tokio::test]
    async fn guest_updates_continue_without_creating_chat_settings() {
        let next: Next = Box::new(|mut request| {
            Box::pin(async move {
                assert!(request.extensions.get::<ChatConfig>().is_none());
                assert!(request.extensions.get::<OwnChatConfig>().is_none());
                request.extensions.insert(ReachedNext);
                Ok(Response::new(request, PropagateEventResult::Unhandled))
            })
        });

        let response = CreateChatMiddleware.call(request("guest_message"), next).await.unwrap();

        assert!(response.request.extensions.get::<ReachedNext>().is_some());
    }

    #[tokio::test]
    async fn unsupported_regular_chats_are_rejected_before_routing() {
        let next: Next = Box::new(|_| Box::pin(async { panic!("Unsupported chats must not reach a handler") }));

        let response = CreateChatMiddleware.call(request("message"), next).await.unwrap();

        assert!(matches!(response.propagate_result, PropagateEventResult::Rejected));
    }

    #[tokio::test]
    async fn callback_chats_use_the_sdk_context_before_resolving_chat_settings() {
        let mut router = Router::new("synthetic")
            .on_update(|observer| observer.register_outer_middleware(CreateChatMiddleware))
            .on_callback_query(|observer| {
                observer.register(Handler::new(|context: Context| async move {
                    let chat = context.get::<TelegramChat>("event_chat").unwrap();
                    assert_eq!(chat.id(), -100);
                    assert!(!matches!(chat, TelegramChat::Private(_)));
                    Ok::<_, Infallible>(EventReturn::Finish)
                }))
            })
            .configure_default();

        for (kind, supported) in [("group", true), ("supergroup", true), ("synthetic_unknown", false)] {
            let update = format!(
                r#"{{"update_id":1,"callback_query":{{"id":"synthetic-query","chat_instance":"synthetic-chat", "from":{{"id":4,"is_bot":false,"first_name":"Synthetic user"}},"data":"open:main","message":{{"message_id":2,"date":1,"chat":{{"id":-100,"type":"{kind}","title":"Synthetic group"}},"text":"Menu"}}}}}}"#
            );
            let request = Request {
                bot: Bot::new("1:synthetic"),
                update: Arc::new(serialization::from_str(&update).unwrap()),
                context: Context::default(),
                extensions: Extensions::default(),
            };
            assert!(request.update.chat().is_none());
            let response = router.propagate_event(UpdateType::CallbackQuery, request).await.unwrap();
            assert_eq!(matches!(response.propagate_result, PropagateEventResult::Handled(_)), supported);
            if !supported {
                assert!(matches!(response.propagate_result, PropagateEventResult::Rejected));
            }
        }
    }
}
