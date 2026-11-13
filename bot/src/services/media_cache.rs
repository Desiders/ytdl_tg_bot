use std::{convert::Infallible, sync::Arc};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::{database::TxManager, entities::CachedMedia, errors::ErrorKind};

pub struct MediaCache {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl MediaCache {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }

    pub async fn get(&self, url: &str, audio_language: &str, max_file_size: i64) -> Result<Option<CachedMedia>, ErrorKind<Infallible>> {
        self.tx_manager
            .media_reader()
            .get_by_url(
                url,
                audio_language,
                max_file_size,
                OffsetDateTime::now_utc() - Duration::minutes(15),
            )
            .await
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<CachedMedia>, ErrorKind<Infallible>> {
        self.tx_manager.media_reader().get(id).await
    }

    pub async fn set(&self, media: CachedMedia) -> Result<CachedMedia, ErrorKind<Infallible>> {
        let tx = self.tx_manager.begin().await?;
        let result = tx.media_repo().replace(media).await;
        match result {
            Ok(media) => {
                tx.commit().await?;
                Ok(media)
            }
            Err(err) => {
                let _ = tx.rollback().await;
                Err(err)
            }
        }
    }
}
