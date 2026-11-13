use async_trait::async_trait;
use std::convert::Infallible;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{entities::MediaCatalog, errors::ErrorKind};

#[async_trait]
pub trait MediaCatalogReader: Send + Sync {
    async fn get_by_url(
        &self,
        url: &str,
        audio_language: &str,
        max_file_size: i64,
        since: OffsetDateTime,
    ) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>>;
    async fn get(&self, id: Uuid) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>>;
}

#[async_trait]
pub trait MediaCatalogRepo: Send + Sync {
    async fn replace(&self, catalog: MediaCatalog) -> Result<MediaCatalog, ErrorKind<Infallible>>;
}
