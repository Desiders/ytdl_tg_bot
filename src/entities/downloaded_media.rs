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
        }: Model,
    ) -> Self {
        Self {
            file_id,
            id,
            display_id,
            domain,
            media_type: media_type.into(),
            created_at,
        }
    }
}
