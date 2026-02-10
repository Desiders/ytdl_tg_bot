use crate::{database::models::downloaded_media::Model, value_objects::MediaType};

use time::OffsetDateTime;

#[derive(Debug)]
pub struct DownloadedMedia {
    pub file_id: String,
    pub id: String,
    pub display_id: Option<String>,
    pub domain: Option<String>,
    pub media_type: MediaType,
    pub created_at: OffsetDateTime,
    pub audio_language: Option<String>,
    pub crop_start_time: Option<i32>,
    pub crop_end_time: Option<i32>,
}

#[derive(Debug)]
pub struct DownloadedMediaCount {
    pub count: i64,
}

#[derive(Debug)]
pub struct DownloadedMediaByDomainCount {
    pub domain: String,
    pub count: i64,
}

#[derive(Debug)]
pub struct DownloadedMediaStats {
    pub last_day: DownloadedMediaCount,
    pub last_week: DownloadedMediaCount,
    pub last_month: DownloadedMediaCount,
    pub total: DownloadedMediaCount,
    pub top_domains: Vec<DownloadedMediaByDomainCount>,
}

impl From<Model> for DownloadedMedia {
    fn from(
        Model {
            file_id,
            id,
            display_id,
            domain,
            media_type,
            created_at,
            audio_language,
            crop_start_time,
            crop_end_time,
        }: Model,
    ) -> Self {
        Self {
            file_id,
            id,
            display_id,
            domain,
            media_type: media_type.into(),
            created_at,
            audio_language,
            crop_start_time,
            crop_end_time,
        }
    }
}
