//! `sea_orm` implementations of [`TxManager`](super::interfaces::tx_manager::TxManager) and
//! [`ActiveTxManager`](super::interfaces::tx_manager::ActiveTxManager).

use async_trait::async_trait;
use sea_orm::{DatabaseConnection, DatabaseTransaction, TransactionTrait as _};
use std::sync::Arc;

use crate::{
    database::{
        factories::TxManagerFactories,
        interfaces::{
            chat::{ChatReader, ChatRepo},
            chat_config::{ChatConfigReader, ChatConfigRepo},
            downloaded_media::{DownloadedMediaReader, DownloadedMediaRepo},
            tx_manager::{ActiveTxManager, TxManager},
        },
    },
    errors::database::{BeginError, CommitError, RollbackError},
};

pub struct SeaOrmTxManager {
    pool: Arc<DatabaseConnection>,
    factories: Arc<TxManagerFactories>,
}

impl SeaOrmTxManager {
    #[must_use]
    pub const fn new(pool: Arc<DatabaseConnection>, factories: Arc<TxManagerFactories>) -> Self {
        Self { pool, factories }
    }
}

pub struct SeaOrmActiveTxManager {
    transaction: DatabaseTransaction,
    factories: Arc<TxManagerFactories>,
}

#[async_trait]
impl TxManager for SeaOrmTxManager {
    async fn begin(&self) -> Result<Box<dyn ActiveTxManager>, BeginError> {
        let transaction = self.pool.begin().await?;
        Ok(Box::new(SeaOrmActiveTxManager {
            transaction,
            factories: self.factories.clone(),
        }))
    }

    fn chat_reader(&self) -> Box<dyn ChatReader + '_> {
        self.factories.chat_reader.create(self.pool.as_ref())
    }

    fn chat_config_reader(&self) -> Box<dyn ChatConfigReader + '_> {
        self.factories.chat_config_reader.create(self.pool.as_ref())
    }

    fn downloaded_media_reader(&self) -> Box<dyn DownloadedMediaReader + '_> {
        self.factories.downloaded_media_reader.create(self.pool.as_ref())
    }
}

#[async_trait]
impl ActiveTxManager for SeaOrmActiveTxManager {
    async fn commit(self: Box<Self>) -> Result<(), CommitError> {
        self.transaction.commit().await?;
        Ok(())
    }

    async fn rollback(self: Box<Self>) -> Result<(), RollbackError> {
        self.transaction.rollback().await?;
        Ok(())
    }

    fn chat_repo(&self) -> Box<dyn ChatRepo + '_> {
        self.factories.chat_repo.create(&self.transaction)
    }

    fn chat_config_repo(&self) -> Box<dyn ChatConfigRepo + '_> {
        self.factories.chat_config_repo.create(&self.transaction)
    }

    fn downloaded_media_repo(&self) -> Box<dyn DownloadedMediaRepo + '_> {
        self.factories.downloaded_media_repo.create(&self.transaction)
    }
}
