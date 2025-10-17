use time::OffsetDateTime;
use uuid::Uuid;

use crate::value_objects::MediaType;

pub struct DownloadedMedia {
    pub id: Uuid,
    pub file_id: Box<str>,
    pub url_or_id: Box<str>,
    pub media_type: MediaType,
    pub created_at: OffsetDateTime,
}
