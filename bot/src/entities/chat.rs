use crate::{database::models::chats::Model, value_objects::ChatType};

use time::OffsetDateTime;

#[derive(Debug)]
pub struct Chat {
    pub tg_id: i64,
    pub username: Option<String>,
    pub chat_type: Option<ChatType>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

impl Chat {
    pub fn new(tg_id: i64, username: Option<String>, chat_type: ChatType) -> Self {
        Self {
            tg_id,
            username,
            chat_type: Some(chat_type),
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }
}

#[derive(Debug)]
pub struct ChatStats {
    pub count: i64,
    pub by_type: Vec<ChatTypeCount>,
}

#[derive(Debug)]
pub struct ChatTypeCount {
    pub chat_type: Option<ChatType>,
    pub count: i64,
}

impl From<Model> for Chat {
    fn from(
        Model {
            tg_id,
            username,
            chat_type,
            created_at,
            updated_at,
        }: Model,
    ) -> Self {
        Self {
            tg_id,
            username,
            chat_type: chat_type.map(Into::into),
            created_at,
            updated_at,
        }
    }
}
