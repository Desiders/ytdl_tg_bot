use froodi::async_impl::Container;
use telers::{
    errors::EventErrorKind,
    event::EventReturn,
    middlewares::outer::{Middleware, MiddlewareResponse},
    types::Chat::Private,
    Request,
};
use tracing::{error, instrument};

use crate::{
    database::TxManager,
    entities::{Chat, ChatConfig},
    interactors::{chat, Interactor as _},
};

#[derive(Clone)]
pub struct CreateChatMiddleware;

impl Middleware for CreateChatMiddleware {
    #[instrument(skip_all)]
    async fn call(&mut self, mut request: Request) -> Result<MiddlewareResponse, EventErrorKind> {
        let (chat_id, cmd_random_enabled, username) = match (request.update.chat(), request.update.from()) {
            (Some(chat), _) => (chat.id(), matches!(chat, Private(_)), chat.username()),
            (None, Some(from)) => (from.id, false, from.username.as_deref()),
            _ => return Ok((request, EventReturn::Finish)),
        };
        let Some(container) = request.extensions.get::<Container>() else {
            return Ok((request, EventReturn::Finish));
        };

        let db_chat = Chat::new(chat_id, username.map(ToOwned::to_owned));
        let db_chat_config = ChatConfig::new(chat_id, cmd_random_enabled);

        let save_chat = container.get::<chat::SaveChat>().await.unwrap();
        let mut tx_manager = container.get_transient::<TxManager>().await.unwrap();

        match save_chat
            .execute(chat::SaveChatInput {
                chat: db_chat,
                chat_config: db_chat_config,
                tx_manager: &mut tx_manager,
            })
            .await
        {
            Ok((_, chat_config, chat_config_exclude_domains)) => {
                request.extensions.insert(chat_config);
                request.extensions.insert(chat_config_exclude_domains);
            }
            Err(err) => error!(%err, "Save chat err"),
        }

        Ok((request, EventReturn::Finish))
    }
}
