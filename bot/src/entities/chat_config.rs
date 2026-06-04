use crate::{
    database::models::{chat_config_exclude_domains, chat_configs},
    locale::Locale,
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatConfig {
    pub tg_id: i64,
    pub cmd_random_enabled: bool,
    pub link_is_visible: bool,
    pub language: String,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
pub struct OwnChatConfig(pub Option<ChatConfig>);

impl ChatConfig {
    pub fn new(tg_id: i64, cmd_random_enabled: bool, language: String) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            link_is_visible: false,
            language,
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    pub fn locale(&self) -> Locale {
        Locale::from(self.language.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ChatConfigUpdate {
    pub tg_id: i64,
    pub cmd_random_enabled: Option<bool>,
    pub link_is_visible: Option<bool>,
    pub language: Option<String>,
    pub updated_at: OffsetDateTime,
}

impl ChatConfigUpdate {
    pub fn new(tg_id: i64) -> Self {
        Self {
            tg_id,
            cmd_random_enabled: None,
            link_is_visible: None,
            language: None,
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    #[allow(dead_code)]
    pub fn with_cmd_random_enabled(mut self, cmd_random_enabled: bool) -> Self {
        self.cmd_random_enabled = Some(cmd_random_enabled);
        self
    }

    pub fn with_link_is_visible(mut self, link_is_visible: bool) -> Self {
        self.link_is_visible = Some(link_is_visible);
        self
    }

    pub fn with_language(mut self, language: String) -> Self {
        self.language = Some(language);
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
            language,
        }: chat_configs::Model,
    ) -> Self {
        Self {
            tg_id,
            cmd_random_enabled,
            link_is_visible,
            language,
            updated_at,
        }
    }
}

impl From<chat_config_exclude_domains::Model> for ChatConfigExcludeDomain {
    fn from(chat_config_exclude_domains::Model { tg_id, domain }: chat_config_exclude_domains::Model) -> Self {
        Self { tg_id, domain }
    }
}
