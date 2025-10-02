use std::convert::Infallible;

use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait};

use crate::{database::models::chat, entities::Chat, errors::database::ErrorKind};

pub struct ChatDao<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> ChatDao<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self
    where
        Conn: ConnectionTrait,
    {
        Self { conn }
    }
}

impl<'a, Conn> ChatDao<'a, Conn>
where
    Conn: ConnectionTrait,
{
    pub async fn insert_or_update(
        &self,
        Chat {
            id,
            tg_id,
            username,
            created_at,
            updated_at,
        }: Chat,
    ) -> Result<Chat, ErrorKind<Infallible>> {
        use chat::{ActiveModel, Column::*, Entity};

        let model = ActiveModel {
            id: Set(id),
            tg_id: Set(tg_id),
            username: Set(username.map(Into::into)),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::column(TgId).update_columns([Username, UpdatedAt]).to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
