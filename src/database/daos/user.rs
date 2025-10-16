use std::convert::Infallible;

use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait};

use crate::{database::models::user, entities::User, errors::database::ErrorKind};

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
        User {
            id,
            tg_id,
            username,
            created_at,
            updated_at,
        }: User,
    ) -> Result<User, ErrorKind<Infallible>> {
        use user::{
            ActiveModel,
            Column::{TgId, UpdatedAt, Username},
            Entity,
        };

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
