use crate::{
    database::TxManager,
    entities::{Chat, ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains, ChatConfigUpdate},
    errors::ErrorKind,
    interactors::Interactor,
};

use std::{convert::Infallible, sync::Arc};
use tracing::{debug, instrument};

pub struct SaveChat {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl SaveChat {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

pub struct SaveChatInput {
    pub chat: Chat,
    pub chat_config: ChatConfig,
}

impl Interactor<SaveChatInput> for &SaveChat {
    type Output = (Chat, ChatConfig, ChatConfigExcludeDomains);
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, SaveChatInput { chat, chat_config }: SaveChatInput) -> Result<Self::Output, Self::Err> {
        let tx = self.tx_manager.begin().await?;

        let saved_chat = tx.chat_repo().insert_or_update(chat).await;
        let chat = match saved_chat {
            Ok(val) => val,
            Err(err) => {
                let _ = tx.rollback().await;
                return Err(err);
            }
        };
        debug!("Chat saved");

        let saved_config = tx.chat_config_repo().insert_or_update(chat_config).await;
        let config = match saved_config {
            Ok(val) => val,
            Err(err) => {
                let _ = tx.rollback().await;
                return Err(err);
            }
        };
        debug!("Chat config saved");

        tx.commit().await?;

        let config_exclude_domains = self.tx_manager.chat_config_reader().get_exclude_domains(chat.tg_id).await?;
        Ok((chat, config, config_exclude_domains))
    }
}

pub struct ExcludeDomainInput {
    pub dto: ChatConfigExcludeDomain,
}

pub struct AddExcludeDomain {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl AddExcludeDomain {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<ExcludeDomainInput> for &AddExcludeDomain {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, ExcludeDomainInput { dto }: ExcludeDomainInput) -> Result<Self::Output, Self::Err> {
        let tx = self.tx_manager.begin().await?;

        let outcome = tx.chat_config_repo().insert_exclude_domain_or_update(dto).await;
        if let Err(err) = outcome {
            let _ = tx.rollback().await;
            return Err(err);
        }
        debug!("Exclude domain saved");

        tx.commit().await?;
        Ok(())
    }
}

pub struct RemoveExcludeDomain {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl RemoveExcludeDomain {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<ExcludeDomainInput> for &RemoveExcludeDomain {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, ExcludeDomainInput { dto }: ExcludeDomainInput) -> Result<Self::Output, Self::Err> {
        let tx = self.tx_manager.begin().await?;

        let outcome = tx.chat_config_repo().delete_exclude_domain(dto).await;
        if let Err(err) = outcome {
            let _ = tx.rollback().await;
            return Err(err);
        }
        debug!("Exclude domain deleted");

        tx.commit().await?;
        Ok(())
    }
}

pub struct UpdateChatConfigInput {
    pub dto: ChatConfigUpdate,
}

pub struct GetChatConfigInput {
    pub tg_id: i64,
}

pub struct GetChatConfig {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetChatConfig {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<GetChatConfigInput> for &GetChatConfig {
    type Output = Option<ChatConfig>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, GetChatConfigInput { tg_id }: GetChatConfigInput) -> Result<Self::Output, Self::Err> {
        let config = self.tx_manager.chat_config_reader().get(tg_id).await?;
        debug!("Chat config fetched");
        Ok(config)
    }
}

pub struct UpdateChatConfig {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl UpdateChatConfig {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<UpdateChatConfigInput> for &UpdateChatConfig {
    type Output = ChatConfig;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, UpdateChatConfigInput { dto }: UpdateChatConfigInput) -> Result<Self::Output, Self::Err> {
        let tx = self.tx_manager.begin().await?;

        let updated = tx.chat_config_repo().update(dto).await;
        let config = match updated {
            Ok(val) => val,
            Err(err) => {
                let _ = tx.rollback().await;
                return Err(err);
            }
        };
        debug!("Chat config updated");

        tx.commit().await?;
        Ok(config)
    }
}
