use time::OffsetDateTime;
use uuid::Uuid;

use crate::entities::MediaFormat;

#[derive(Debug, Clone)]
pub struct MediaCatalog {
    pub id: Uuid,
    pub url: String,
    pub audio_language: String,
    pub max_file_size: i64,
    pub media_id: String,
    pub webpage_url: String,
    pub title: String,
    pub description: Option<String>,
    pub duration: Option<f32>,
    pub updated_at: OffsetDateTime,
    pub formats: Vec<MediaCatalogFormat>,
}

#[derive(Debug, Clone)]
pub struct MediaCatalogFormat {
    pub id: i64,
    pub format: MediaFormat,
}
