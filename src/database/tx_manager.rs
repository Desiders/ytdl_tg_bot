use sea_orm::{AccessMode, DatabaseConnection, DatabaseTransaction, IsolationLevel, TransactionTrait as _};
use std::sync::Arc;

use crate::{
    database::{ChatDao, DownloadedMediaDao, UserDao, UserDownloadedMediaDao},
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

    pub fn user_dao(&self) -> Result<UserDao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(UserDao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }

    pub fn chat_dao(&self) -> Result<ChatDao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(ChatDao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }

    pub fn downloaded_media_dao(&self) -> Result<DownloadedMediaDao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(DownloadedMediaDao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }

    pub fn user_downloaded_media_dao(&self) -> Result<UserDownloadedMediaDao<DatabaseTransaction>, TransactionNotBegin> {
        Ok(UserDownloadedMediaDao::new(self.transaction.as_ref().ok_or(TransactionNotBegin)?))
    }
}
