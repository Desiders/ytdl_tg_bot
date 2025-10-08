use super::Interactor;
use crate::{
    database::TxManager,
    entities::{Chat, User},
    errors::database::ErrorKind,
};

use std::convert::Infallible;
use tracing::{event, span, Level};

pub struct CreateUserAndChat<'a> {
    pub tx_manager: &'a mut TxManager,
}

impl<'a> CreateUserAndChat<'a> {
    pub const fn new(tx_manager: &'a mut TxManager) -> Self {
        Self { tx_manager }
    }
}

pub struct CreateUserAndChatInput {
    pub user: User,
    pub chat: Chat,
}

pub struct CreateUserAndChatOutput {
    pub user: User,
    pub chat: Chat,
}

impl Interactor for CreateUserAndChat<'_> {
    type Input<'a> = CreateUserAndChatInput;
    type Output = CreateUserAndChatOutput;
    type Err = ErrorKind<Infallible>;

    async fn execute(&mut self, Self::Input { user, chat }: Self::Input<'_>) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "create_user_and_chat");
        let _enter = span.enter();

        self.tx_manager.begin().await?;

        let user = match self.tx_manager.user_dao()?.insert_or_update(user).await {
            Ok(val) => val,
            Err(err) => {
                self.tx_manager.rollback().await?;
                return Err(err);
            }
        };
        event!(Level::INFO, "User created");

        let chat = match self.tx_manager.chat_dao()?.insert_or_update(chat).await {
            Ok(val) => val,
            Err(err) => {
                self.tx_manager.rollback().await?;
                return Err(err);
            }
        };
        event!(Level::INFO, "Chat created");

        self.tx_manager.commit().await?;

        Ok(Self::Output { user, chat })
    }
}
