use froodi::async_impl::Container;
use telers::{
    errors::EventErrorKind,
    event::EventReturn,
    middlewares::outer::{Middleware, MiddlewareResponse},
    Request,
};
use tracing::{event, instrument, Level};

use crate::{
    database::TxManager,
    entities::Chat,
    interactors::{Interactor as _, SaveChat, SaveChatInput},
};

#[derive(Clone)]
pub struct CreateChatMiddleware;

impl Middleware for CreateChatMiddleware {
    #[instrument(skip_all)]
    async fn call(&mut self, request: Request) -> Result<MiddlewareResponse, EventErrorKind> {
        let Some(chat) = request.update.chat() else {
            return Ok((request, EventReturn::Finish));
        };
        let Some(container) = request.extensions.get::<Container>() else {
            return Ok((request, EventReturn::Finish));
        };

        let chat_id = chat.id();
        let username = chat.username();

        let db_chat = Chat::new(chat_id, username.map(ToOwned::to_owned));

        let save_chat = container.get::<SaveChat>().await.unwrap();
        let mut tx_manager = container.get_transient::<TxManager>().await.unwrap();

        if let Err(err) = save_chat.execute(SaveChatInput::new(db_chat, &mut tx_manager)).await {
            event!(Level::ERROR, %err, "Save chat err");
        }

        Ok((request, EventReturn::Finish))
    }
}
