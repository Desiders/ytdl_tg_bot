use async_trait::async_trait;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use std::convert::Infallible;

use crate::{
    database::{
        interfaces::chat_config::ChatConfigReader,
        models::{chat_config_exclude_domains, chat_configs},
    },
    entities::{ChatConfig, ChatConfigExcludeDomains},
    errors::ErrorKind,
};

pub struct SeaOrmChatConfigReader<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmChatConfigReader<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> ChatConfigReader for SeaOrmChatConfigReader<'_, Conn> {
    async fn get(&self, tg_id: i64) -> Result<Option<ChatConfig>, ErrorKind<Infallible>> {
        use chat_configs::Entity;

        Entity::find_by_id(tg_id)
            .one(self.conn)
            .await
            .map(|row| row.map(Into::into))
            .map_err(Into::into)
    }

    async fn get_exclude_domains(&self, tg_id: i64) -> Result<ChatConfigExcludeDomains, ErrorKind<Infallible>> {
        use chat_config_exclude_domains::{Column::TgId, Entity};

        Entity::find()
            .filter(TgId.eq(tg_id))
            .all(self.conn)
            .await
            .map(|rows| ChatConfigExcludeDomains(rows.into_iter().map(|row| row.domain).collect()))
            .map_err(Into::into)
    }
}
