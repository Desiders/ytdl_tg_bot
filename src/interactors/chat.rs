use super::Interactor;
use crate::{
    database::TxManager,
    entities::{Chat, ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains, ChatConfigUpdate},
    errors::ErrorKind,
};

use std::convert::Infallible;
use tracing::{debug, instrument};

pub struct SaveChat {}

pub struct SaveChatInput<'a> {
    pub chat: Chat,
    pub chat_config: ChatConfig,
    pub tx_manager: &'a mut TxManager,
}

impl Interactor<SaveChatInput<'_>> for &SaveChat {
    type Output = (Chat, ChatConfig, ChatConfigExcludeDomains);
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
        let config_exclude_domains = dao.get_exclude_domains(chat.tg_id).await?;

        tx_manager.commit().await?;
        Ok((chat, config, config_exclude_domains))
    }
}

pub struct ExcludeDomainInput<'a> {
    pub dto: ChatConfigExcludeDomain,
    pub tx_manager: &'a mut TxManager,
}

pub struct AddExcludeDomain {}

impl Interactor<ExcludeDomainInput<'_>> for &AddExcludeDomain {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, ExcludeDomainInput { dto, tx_manager }: ExcludeDomainInput<'_>) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.chat_config_dao().unwrap();
        let _ = dao.insert_exclude_domain_or_update(dto).await?;
        debug!("Exclude domain saved");

        tx_manager.commit().await?;
        Ok(())
    }
}

pub struct RemoveExcludeDomain {}

impl Interactor<ExcludeDomainInput<'_>> for &RemoveExcludeDomain {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, ExcludeDomainInput { dto, tx_manager }: ExcludeDomainInput<'_>) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.chat_config_dao().unwrap();
        let _ = dao.delete_exclude_domain(dto).await?;
        debug!("Exclude domain deleted");

        tx_manager.commit().await?;
        Ok(())
    }
}

pub struct UpdateChatConfigInput<'a> {
    pub dto: ChatConfigUpdate,
    pub tx_manager: &'a mut TxManager,
}

pub struct UpdateChatConfig {}

impl Interactor<UpdateChatConfigInput<'_>> for &UpdateChatConfig {
    type Output = ChatConfig;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, UpdateChatConfigInput { dto, tx_manager }: UpdateChatConfigInput<'_>) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.chat_config_dao().unwrap();
        let config = dao.update(dto).await?;
        debug!("Chat config updated");

        tx_manager.commit().await?;
        Ok(config)
    }
}
