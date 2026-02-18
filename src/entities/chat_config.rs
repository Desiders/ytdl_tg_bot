use crate::database::models::{chat_config_exclude_domains, chat_configs};

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

#[derive(Debug, Clone)]
pub struct ChatConfigExcludeDomain {
    pub tg_id: i64,
    pub domain: String,
}

impl ChatConfigExcludeDomain {
    pub fn new(tg_id: i64, domain: String) -> Self {
        Self { tg_id, domain }
    }
}

#[derive(Debug, Clone)]
pub struct ChatConfigExcludeDomains(pub Vec<String>);

impl From<chat_configs::Model> for ChatConfig {
    fn from(
        chat_configs::Model {
            tg_id,
            cmd_random_enabled,
            updated_at,
        }: chat_configs::Model,
    ) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            updated_at,
        }
    }
}

impl From<chat_config_exclude_domains::Model> for ChatConfigExcludeDomain {
    fn from(chat_config_exclude_domains::Model { tg_id, domain }: chat_config_exclude_domains::Model) -> Self {
        Self { tg_id, domain }
    }
}
