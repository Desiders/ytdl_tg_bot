use std::convert::Infallible;

use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait};

use crate::{database::models::chats, entities::Chat, errors::ErrorKind};

pub struct Dao<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> Dao<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self
    where
        Conn: ConnectionTrait,
    {
        Self { conn }
    }
}

impl<Conn> Dao<'_, Conn>
where
    Conn: ConnectionTrait,
{
    pub async fn insert_or_update(
        &self,
        Chat {
            tg_id,
            username,
            created_at,
            updated_at,
        }: Chat,
    ) -> Result<(), ErrorKind<Infallible>> {
        use chats::{
            ActiveModel,
            Column::{TgId, UpdatedAt, Username},
            Entity,
        };

        let model = ActiveModel {
            tg_id: Set(tg_id),
            username: Set(username),
            created_at: Set(created_at),
            updated_at: Set(updated_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::column(TgId).update_columns([Username, UpdatedAt]).to_owned())
            .exec_without_returning(self.conn)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}
