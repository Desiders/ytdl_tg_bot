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
    interactors::{Interactor as _, SaveChat, SaveChatInput},
};

#[derive(Clone)]
pub struct CreateChatMiddleware;

impl Middleware for CreateChatMiddleware {
    #[instrument(skip_all)]
    async fn call(&mut self, mut request: Request) -> Result<MiddlewareResponse, EventErrorKind> {
        let Some(chat) = request.update.chat() else {
            return Ok((request, EventReturn::Finish));
        };
        let Some(container) = request.extensions.get::<Container>() else {
            return Ok((request, EventReturn::Finish));
        };

        let chat_id = chat.id();
        let username = chat.username();
        let cmd_random_enabled = matches!(chat, Private(_));

        let db_chat = Chat::new(chat_id, username.map(ToOwned::to_owned));
        let db_chat_config = ChatConfig::new(chat_id, cmd_random_enabled);

        let save_chat = container.get::<SaveChat>().await.unwrap();
        let mut tx_manager = container.get_transient::<TxManager>().await.unwrap();

        match save_chat
            .execute(SaveChatInput::new(db_chat, db_chat_config, &mut tx_manager))
            .await
        {
            Ok((_, chat_config)) => {
                request.extensions.insert(chat_config);
            }
            Err(err) => error!(%err, "Save chat err"),
        }

        Ok((request, EventReturn::Finish))
    }
}
