use crate::database::models::chat::Model;

use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug)]
pub struct Chat {
    pub id: Uuid,
    pub tg_id: i64,
    pub username: Option<Box<str>>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Chat {
    pub fn new(tg_id: i64, username: Option<Box<str>>) -> Self {
        Self {
            id: Uuid::now_v7(),
            tg_id,
            username,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }
}

impl From<Model> for Chat {
    fn from(
        Model {
            id,
            tg_id,
            username,
            created_at,
            updated_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            tg_id,
            username: username.map(String::into_boxed_str),
            created_at,
            updated_at,
        }
    }
}
