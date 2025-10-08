use time::OffsetDateTime;
use uuid::Uuid;

pub struct UserDownloadedMedia {
    pub id: Uuid,
    pub user_id: Uuid,
    pub chat_id: Option<Uuid>,
    pub user_downloaded_media: Uuid,
    pub created_at: OffsetDateTime,
}
