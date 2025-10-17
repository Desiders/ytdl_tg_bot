use crate::{database::models::downloaded_media::Model, value_objects::MediaType};

use time::OffsetDateTime;

pub struct DownloadedMedia {
    pub file_id: String,
    pub url_or_id: String,
    pub media_type: MediaType,
    pub chat_tg_id: i64,
    pub created_at: OffsetDateTime,
}

impl From<Model> for DownloadedMedia {
    fn from(
        Model {
            file_id,
            url_or_id,
            media_type,
            chat_tg_id,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            file_id,
            url_or_id,
            media_type: media_type.into(),
            chat_tg_id,
            created_at,
        }
    }
}
