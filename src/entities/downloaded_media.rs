use crate::{database::models::downloaded_media::Model, value_objects::MediaType};

use time::OffsetDateTime;

pub struct DownloadedMedia {
    pub file_id: String,
    pub id: String,
    pub domain: Option<String>,
    pub media_type: MediaType,
    pub chat_tg_id: i64,
    pub created_at: OffsetDateTime,
}

impl From<Model> for DownloadedMedia {
    fn from(
        Model {
            file_id,
            id,
            domain,
            media_type,
            chat_tg_id,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            file_id,
            id,
            domain,
            media_type: media_type.into(),
            chat_tg_id,
            created_at,
        }
    }
}
