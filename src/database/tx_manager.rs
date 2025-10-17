use sea_orm::{AccessMode, DatabaseConnection, DatabaseTransaction, IsolationLevel, TransactionTrait as _};
use std::sync::Arc;

use crate::{
    database::daos::{chat, chat_downloaded_media, downloaded_media, user},
    errors::database::{BeginError, CommitError, RollbackError, TransactionNotBegin},
};

pub struct TxManager {
    pool: Arc<DatabaseConnection>,
    transaction: Option<DatabaseTransaction>,
}

impl TxManager {
    pub const fn new(pool: Arc<DatabaseConnection>) -> Self {
        Self { pool, transaction: None }
    }
}

impl TxManager {
    pub async fn begin(&mut self) -> Result<(), BeginError> {
        if self.transaction.is_none() {
            self.transaction = Some(self.pool.begin().await?);
        }
        Ok(())
    }

    pub async fn begin_with_config(&mut self, level: IsolationLevel, access_mode: AccessMode) -> Result<(), BeginError> {
        if self.transaction.is_none() {
            self.transaction = Some(self.pool.begin_with_config(Some(level), Some(access_mode)).await?);
        }
        Ok(())
    }

    pub async fn commit(&mut self) -> Result<(), CommitError> {
        if let Some(transaction) = self.transaction.take() {
            transaction.commit().await?;
        }
        Ok(())
    }

    pub async fn rollback(&mut self) -> Result<(), RollbackError> {
        if let Some(transaction) = self.transaction.take() {
            transaction.rollback().await?;
        }
        Ok(())
    }

    #[inline]
    pub fn user_dao(&self) -> Result<user::Dao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(user::Dao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }

    #[inline]
    pub fn chat_dao(&self) -> Result<chat::Dao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(chat::Dao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }

    #[inline]
    pub fn downloaded_media_dao(&self) -> Result<downloaded_media::Dao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(downloaded_media::Dao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }

    #[inline]
    pub fn chat_downloaded_media_dao(&self) -> Result<chat_downloaded_media::Dao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(chat_downloaded_media::Dao::new(
            self.transaction.as_ref().ok_or(TransactionNotBegin)?,
        ))
    }
}
