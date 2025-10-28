use super::Interactor;
use crate::{database::TxManager, entities::Chat, errors::database::ErrorKind};

use std::convert::Infallible;
use tracing::{event, instrument, Level};

pub struct CreateChat {}

impl CreateChat {
    pub const fn new() -> Self {
        Self {}
    }
}

pub struct CreateChatInput<'a> {
    pub chat: Chat,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> CreateChatInput<'a> {
    pub const fn new(chat: Chat, tx_manager: &'a mut TxManager) -> Self {
        Self { chat, tx_manager }
    }
}

impl Interactor<CreateChatInput<'_>> for &CreateChat {
    type Output = Chat;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, CreateChatInput { chat, tx_manager }: CreateChatInput<'_>) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.chat_dao().unwrap();
        let chat = match dao.insert_or_update(chat).await {
            Ok(val) => val,
            Err(err) => {
                tx_manager.rollback().await?;
                return Err(err);
            }
        };
        event!(Level::INFO, "Chat created");

        tx_manager.commit().await?;
        Ok(chat)
    }
}
