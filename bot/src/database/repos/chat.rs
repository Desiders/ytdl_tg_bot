use async_trait::async_trait;
use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait as _};
use std::convert::Infallible;

use crate::{
    database::{interfaces::chat::ChatRepo, models::chats},
    entities::Chat,
    errors::ErrorKind,
};

pub struct SeaOrmChatRepo<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmChatRepo<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> ChatRepo for SeaOrmChatRepo<'_, Conn> {
    async fn insert_or_update(
        &self,
        Chat {
            tg_id,
            username,
            kind,
            created_at,
            updated_at,
        }: Chat,
    ) -> Result<Chat, ErrorKind<Infallible>> {
        use chats::{
            ActiveModel,
            Column::{ChatType, TgId, UpdatedAt, Username},
            Entity,
        };

        let model = ActiveModel {
            tg_id: Set(tg_id),
            username: Set(username),
            chat_type: Set(kind.map(Into::into)),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::column(TgId).update_columns([Username, ChatType, UpdatedAt]).to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
