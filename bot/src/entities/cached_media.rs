use time::OffsetDateTime;
use uuid::Uuid;

use crate::entities::MediaFormat;

#[derive(Debug, Clone)]
pub struct CachedMedia {
    pub id: Uuid,
    pub url: String,
    pub audio_language: String,
    pub max_file_size: i64,
    pub title: String,
    pub description: Option<String>,
    pub updated_at: OffsetDateTime,
    pub formats: Vec<CachedMediaFormat>,
}

#[derive(Debug, Clone)]
pub struct CachedMediaFormat {
    pub id: i64,
    pub format: MediaFormat,
}
