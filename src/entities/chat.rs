use crate::database::models::chats::Model;

use time::OffsetDateTime;

#[derive(Debug)]
pub struct Chat {
    pub tg_id: i64,
    pub username: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Chat {
    pub fn new(tg_id: i64, username: Option<String>) -> Self {
        Self {
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
            tg_id,
            username,
            created_at,
            updated_at,
        }: Model,
    ) -> Self {
        Self {
            tg_id,
            username,
            created_at,
            updated_at,
        }
    }
}
