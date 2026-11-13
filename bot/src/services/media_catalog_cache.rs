use std::{convert::Infallible, sync::Arc};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{database::TxManager, entities::MediaCatalog, errors::ErrorKind};

pub struct MediaCatalogCache {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl MediaCatalogCache {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }

    pub async fn get(&self, url: &str, audio_language: &str, max_file_size: i64) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>> {
        self.tx_manager
            .media_catalog_reader()
            .get_by_url(
                url,
                audio_language,
                max_file_size,
                OffsetDateTime::now_utc() - Duration::minutes(15),
            )
            .await
    }

    pub async fn get_catalog(&self, id: Uuid) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>> {
        self.tx_manager.media_catalog_reader().get(id).await
    }

    pub async fn set(&self, catalog: MediaCatalog) -> Result<MediaCatalog, ErrorKind<Infallible>> {
        let tx = self.tx_manager.begin().await?;
        let result = tx.media_catalog_repo().replace(catalog).await;
        match result {
            Ok(catalog) => {
                tx.commit().await?;
                Ok(catalog)
            }
            Err(err) => {
                let _ = tx.rollback().await;
                Err(err)
            }
        }
    }
}
