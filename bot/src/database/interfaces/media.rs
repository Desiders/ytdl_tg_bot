use async_trait::async_trait;
use std::convert::Infallible;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{entities::CachedMedia, errors::ErrorKind};

#[async_trait]
pub trait MediaReader: Send + Sync {
    async fn get_by_url(
        &self,
        url: &str,
        audio_language: &str,
        max_file_size: i64,
        since: OffsetDateTime,
    ) -> Result<Option<CachedMedia>, ErrorKind<Infallible>>;
    async fn get(&self, id: Uuid) -> Result<Option<CachedMedia>, ErrorKind<Infallible>>;
}

#[async_trait]
pub trait MediaRepo: Send + Sync {
    async fn replace(&self, media: CachedMedia) -> Result<CachedMedia, ErrorKind<Infallible>>;
}
