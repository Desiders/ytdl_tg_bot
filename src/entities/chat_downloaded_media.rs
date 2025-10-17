use time::OffsetDateTime;
use uuid::Uuid;

pub struct ChatDownloadedMedia {
    pub id: Uuid,
    pub chat_id: Uuid,
    pub downloaded_media: Uuid,
    pub created_at: OffsetDateTime,
}
