use crate::{database::models::downloaded_media::Model, value_objects::MediaType};

use time::OffsetDateTime;
use uuid::Uuid;

pub struct DownloadedMedia {
    pub id: Uuid,
    pub file_id: Box<str>,
    pub url_or_id: Box<str>,
    pub media_type: MediaType,
    pub created_at: OffsetDateTime,
}

impl From<Model> for DownloadedMedia {
    fn from(
        Model {
            id,
            file_id,
            url_or_id,
            media_type,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            file_id: file_id.into_boxed_str(),
            url_or_id: url_or_id.into_boxed_str(),
            media_type: media_type.into(),
            created_at,
        }
    }
}
