//! Factories that build readers/repos for a specific connection or transaction.
//!
//! A factory is cheap to store in DI; each `create` call builds the concrete data-access object
//! for the given connection. `TxManagerFactories` is the collection injected into
//! [`SeaOrmTxManager`](super::tx_manager::SeaOrmTxManager) — extend it when adding an entity.

use sea_orm::{DatabaseConnection, DatabaseTransaction};

use crate::database::{
    interfaces::{
        chat::{ChatReader, ChatRepo},
        chat_config::{ChatConfigReader, ChatConfigRepo},
        downloaded_media::{DownloadedMediaReader, DownloadedMediaRepo},
    },
    readers::{chat::SeaOrmChatReader, chat_config::SeaOrmChatConfigReader, downloaded_media::SeaOrmDownloadedMediaReader},
    repos::{chat::SeaOrmChatRepo, chat_config::SeaOrmChatConfigRepo, downloaded_media::SeaOrmDownloadedMediaRepo},
};

/// Creates a reader trait object from the shared database connection.
pub trait ChatReaderFactory: Send + Sync {
    fn create<'a>(&self, conn: &'a DatabaseConnection) -> Box<dyn ChatReader + 'a>;
}

/// Creates a repository trait object from the active database transaction.
pub trait ChatRepoFactory: Send + Sync {
    fn create<'a>(&self, conn: &'a DatabaseTransaction) -> Box<dyn ChatRepo + 'a>;
}

pub trait ChatConfigReaderFactory: Send + Sync {
    fn create<'a>(&self, conn: &'a DatabaseConnection) -> Box<dyn ChatConfigReader + 'a>;
}

pub trait ChatConfigRepoFactory: Send + Sync {
    fn create<'a>(&self, conn: &'a DatabaseTransaction) -> Box<dyn ChatConfigRepo + 'a>;
}

pub trait DownloadedMediaReaderFactory: Send + Sync {
    fn create<'a>(&self, conn: &'a DatabaseConnection) -> Box<dyn DownloadedMediaReader + 'a>;
}

pub trait DownloadedMediaRepoFactory: Send + Sync {
    fn create<'a>(&self, conn: &'a DatabaseTransaction) -> Box<dyn DownloadedMediaRepo + 'a>;
}

/// Factory collection injected into `SeaOrmTxManager`. Add a field per entity reader/repo.
pub struct TxManagerFactories {
    pub chat_reader: Box<dyn ChatReaderFactory>,
    pub chat_repo: Box<dyn ChatRepoFactory>,
    pub chat_config_reader: Box<dyn ChatConfigReaderFactory>,
    pub chat_config_repo: Box<dyn ChatConfigRepoFactory>,
    pub downloaded_media_reader: Box<dyn DownloadedMediaReaderFactory>,
    pub downloaded_media_repo: Box<dyn DownloadedMediaRepoFactory>,
}

impl TxManagerFactories {
    #[must_use]
    pub fn new(
        chat_reader: Box<dyn ChatReaderFactory>,
        chat_repo: Box<dyn ChatRepoFactory>,
        chat_config_reader: Box<dyn ChatConfigReaderFactory>,
        chat_config_repo: Box<dyn ChatConfigRepoFactory>,
        downloaded_media_reader: Box<dyn DownloadedMediaReaderFactory>,
        downloaded_media_repo: Box<dyn DownloadedMediaRepoFactory>,
    ) -> Self {
        Self {
            chat_reader,
            chat_repo,
            chat_config_reader,
            chat_config_repo,
            downloaded_media_reader,
            downloaded_media_repo,
        }
    }
}

#[derive(Clone)]
pub struct DefaultChatReaderFactory;
impl ChatReaderFactory for DefaultChatReaderFactory {
    fn create<'a>(&self, conn: &'a DatabaseConnection) -> Box<dyn ChatReader + 'a> {
        Box::new(SeaOrmChatReader::new(conn))
    }
}

#[derive(Clone)]
pub struct DefaultChatRepoFactory;
impl ChatRepoFactory for DefaultChatRepoFactory {
    fn create<'a>(&self, conn: &'a DatabaseTransaction) -> Box<dyn ChatRepo + 'a> {
        Box::new(SeaOrmChatRepo::new(conn))
    }
}

#[derive(Clone)]
pub struct DefaultChatConfigReaderFactory;
impl ChatConfigReaderFactory for DefaultChatConfigReaderFactory {
    fn create<'a>(&self, conn: &'a DatabaseConnection) -> Box<dyn ChatConfigReader + 'a> {
        Box::new(SeaOrmChatConfigReader::new(conn))
    }
}

#[derive(Clone)]
pub struct DefaultChatConfigRepoFactory;
impl ChatConfigRepoFactory for DefaultChatConfigRepoFactory {
    fn create<'a>(&self, conn: &'a DatabaseTransaction) -> Box<dyn ChatConfigRepo + 'a> {
        Box::new(SeaOrmChatConfigRepo::new(conn))
    }
}

#[derive(Clone)]
pub struct DefaultDownloadedMediaReaderFactory;
impl DownloadedMediaReaderFactory for DefaultDownloadedMediaReaderFactory {
    fn create<'a>(&self, conn: &'a DatabaseConnection) -> Box<dyn DownloadedMediaReader + 'a> {
        Box::new(SeaOrmDownloadedMediaReader::new(conn))
    }
}

#[derive(Clone)]
pub struct DefaultDownloadedMediaRepoFactory;
impl DownloadedMediaRepoFactory for DefaultDownloadedMediaRepoFactory {
    fn create<'a>(&self, conn: &'a DatabaseTransaction) -> Box<dyn DownloadedMediaRepo + 'a> {
        Box::new(SeaOrmDownloadedMediaRepo::new(conn))
    }
}

impl Default for TxManagerFactories {
    fn default() -> Self {
        Self::new(
            Box::new(DefaultChatReaderFactory),
            Box::new(DefaultChatRepoFactory),
            Box::new(DefaultChatConfigReaderFactory),
            Box::new(DefaultChatConfigRepoFactory),
            Box::new(DefaultDownloadedMediaReaderFactory),
            Box::new(DefaultDownloadedMediaRepoFactory),
        )
    }
}
