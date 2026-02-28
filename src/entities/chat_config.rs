use crate::database::models::{chat_config_exclude_domains, chat_configs};

use time::OffsetDateTime;

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub tg_id: i64,
    pub cmd_random_enabled: bool,
    pub link_is_visible: bool,
    pub updated_at: OffsetDateTime,
}

impl ChatConfig {
    pub fn new(tg_id: i64, cmd_random_enabled: bool) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            link_is_visible: false,
            updated_at: OffsetDateTime::now_utc(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatConfigUpdate {
    pub tg_id: i64,
    pub cmd_random_enabled: Option<bool>,
    pub link_is_visible: Option<bool>,
    pub updated_at: OffsetDateTime,
}

impl ChatConfigUpdate {
    pub fn new(tg_id: i64) -> Self {
        Self {
            tg_id,
            cmd_random_enabled: None,
            link_is_visible: None,
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    pub fn with_cmd_random_enabled(mut self, cmd_random_enabled: bool) -> Self {
        self.cmd_random_enabled = Some(cmd_random_enabled);
        self
    }

    pub fn with_link_is_visible(mut self, link_is_visible: bool) -> Self {
        self.link_is_visible = Some(link_is_visible);
        self
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
            link_is_visible,
        }: chat_configs::Model,
    ) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            updated_at,
            link_is_visible,
        }
    }
}

impl From<chat_config_exclude_domains::Model> for ChatConfigExcludeDomain {
    fn from(chat_config_exclude_domains::Model { tg_id, domain }: chat_config_exclude_domains::Model) -> Self {
        Self { tg_id, domain }
    }
}
