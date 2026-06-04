//! Transaction-manager interfaces.
//!
//! `TxManager` is the stable object injected by DI: it creates readers from the pool (no active
//! transaction) and can `begin()` a transaction. `ActiveTxManager` owns one active transaction and
//! creates repos scoped to it, so the transaction lifetime is tied to the local owner. The
//! `sea_orm` implementations live in [`super::super::tx_manager`].

use async_trait::async_trait;

use crate::{
    database::interfaces::{
        chat::{ChatReader, ChatRepo},
        chat_config::{ChatConfigReader, ChatConfigRepo},
        downloaded_media::{DownloadedMediaReader, DownloadedMediaRepo},
    },
    errors::database::{BeginError, CommitError, RollbackError},
};

/// Starts transactions and creates readers that do not need an active transaction.
#[async_trait]
pub trait TxManager: Send + Sync {
    async fn begin(&self) -> Result<Box<dyn ActiveTxManager>, BeginError>;

    fn chat_reader(&self) -> Box<dyn ChatReader + '_>;
    fn chat_config_reader(&self) -> Box<dyn ChatConfigReader + '_>;
    fn downloaded_media_reader(&self) -> Box<dyn DownloadedMediaReader + '_>;
}

/// Owns an active transaction and creates repos scoped to it.
#[async_trait]
pub trait ActiveTxManager: Send {
    async fn commit(self: Box<Self>) -> Result<(), CommitError>;
    async fn rollback(self: Box<Self>) -> Result<(), RollbackError>;

    fn chat_repo(&self) -> Box<dyn ChatRepo + '_>;
    fn chat_config_repo(&self) -> Box<dyn ChatConfigRepo + '_>;
    fn downloaded_media_repo(&self) -> Box<dyn DownloadedMediaRepo + '_>;
}
