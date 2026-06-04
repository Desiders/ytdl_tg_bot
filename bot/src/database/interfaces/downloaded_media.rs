use async_trait::async_trait;
use std::convert::Infallible;

use crate::{
    entities::{DownloadedMedia, DownloadedMediaStats},
    errors::ErrorKind,
    value_objects::MediaType,
};

#[async_trait]
pub trait DownloadedMediaReader: Send + Sync {
    async fn get(
        &self,
        search: &str,
        domain: Option<&str>,
        audio_language: Option<&str>,
        media_type: MediaType,
        crop_start_time: Option<i32>,
        crop_end_time: Option<i32>,
    ) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>>;
    async fn get_random(
        &self,
        limit: u64,
        media_type: MediaType,
        domains: &[String],
    ) -> Result<Vec<DownloadedMedia>, ErrorKind<Infallible>>;
    async fn get_stats(&self, top_domains_limit: u64) -> Result<DownloadedMediaStats, ErrorKind<Infallible>>;
}

#[async_trait]
pub trait DownloadedMediaRepo: Send + Sync {
    async fn insert_or_ignore(&self, media: DownloadedMedia) -> Result<(), ErrorKind<Infallible>>;
    async fn insert_or_replace(&self, media: DownloadedMedia) -> Result<(), ErrorKind<Infallible>>;
}
