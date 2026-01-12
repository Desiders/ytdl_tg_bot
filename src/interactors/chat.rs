use super::Interactor;
use crate::{
    database::TxManager,
    entities::{Chat, ChatConfig},
    errors::ErrorKind,
};

use std::convert::Infallible;
use tracing::{debug, instrument};

pub struct SaveChat {}

impl SaveChat {
    pub const fn new() -> Self {
        Self {}
    }
}

pub struct SaveChatInput<'a> {
    pub chat: Chat,
    pub chat_config: ChatConfig,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> SaveChatInput<'a> {
    pub const fn new(chat: Chat, chat_config: ChatConfig, tx_manager: &'a mut TxManager) -> Self {
        Self {
            chat,
            chat_config,
            tx_manager,
        }
    }
}

impl Interactor<SaveChatInput<'_>> for &SaveChat {
    type Output = (Chat, ChatConfig);
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SaveChatInput {
            chat,
            chat_config,
            tx_manager,
        }: SaveChatInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.chat_dao().unwrap();
        let chat = match dao.insert_or_update(chat).await {
            Ok(val) => val,
            Err(err) => {
                tx_manager.rollback().await?;
                return Err(err);
            }
        };
        debug!("Chat saved");
        let dao = tx_manager.chat_config_dao().unwrap();
        let config = match dao.insert_or_update(chat_config).await {
            Ok(val) => val,
            Err(err) => {
                tx_manager.rollback().await?;
                return Err(err);
            }
        };
        debug!("Chat config saved");

        tx_manager.commit().await?;
        Ok((chat, config))
    }
}
