use super::Interactor;
use crate::{database::TxManager, entities::Chat, errors::ErrorKind};

use std::convert::Infallible;
use tracing::{info, instrument};

pub struct SaveChat {}

impl SaveChat {
    pub const fn new() -> Self {
        Self {}
    }
}

pub struct SaveChatInput<'a> {
    pub chat: Chat,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> SaveChatInput<'a> {
    pub const fn new(chat: Chat, tx_manager: &'a mut TxManager) -> Self {
        Self { chat, tx_manager }
    }
}

impl Interactor<SaveChatInput<'_>> for &SaveChat {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, SaveChatInput { chat, tx_manager }: SaveChatInput<'_>) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.chat_dao().unwrap();
        match dao.insert_or_update(chat).await {
            Ok(val) => val,
            Err(err) => {
                tx_manager.rollback().await?;
                return Err(err);
            }
        };
        info!("Chat saved");

        tx_manager.commit().await?;
        Ok(())
    }
}
