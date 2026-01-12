use crate::database::models::chat_configs::Model;

use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub tg_id: i64,
    pub cmd_random_enabled: bool,
    pub updated_at: OffsetDateTime,
}

impl ChatConfig {
    pub fn new(tg_id: i64, cmd_random_enabled: bool) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            updated_at: OffsetDateTime::now_utc(),
        }
    }
}

impl From<Model> for ChatConfig {
    fn from(
        Model {
            tg_id,
            cmd_random_enabled,
            updated_at,
        }: Model,
    ) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            updated_at,
        }
    }
}
