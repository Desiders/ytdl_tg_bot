use crate::database::models::chat_downloaded_media::Model;

use time::OffsetDateTime;
use uuid::Uuid;

pub struct ChatDownloadedMedia {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub downloaded_media: Uuid,
    pub created_at: OffsetDateTime,
}

impl From<Model> for ChatDownloadedMedia {
    fn from(
        Model {
            id,
            chat_id,
            downloaded_media,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            chat_id,
            downloaded_media,
            created_at,
        }
    }
}
