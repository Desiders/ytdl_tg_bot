use time::OffsetDateTime;
use uuid::Uuid;

use crate::value_objects::MediaType;

pub struct DownloadedMedia {
    pub id: Uuid,
    pub tg_id: i64,
    pub url: Box<str>,
    pub id_in_url: Option<Box<str>>,
    pub media_type: MediaType,
    pub created_at: OffsetDateTime,
}
