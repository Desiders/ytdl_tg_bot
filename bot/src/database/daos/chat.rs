use sea_orm::{
    prelude::Expr, sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait, ExprTrait as _, FromQueryResult, QuerySelect as _,
};
use std::convert::Infallible;

use crate::{
    database::models::chats,
    entities::{Chat, ChatStats},
    errors::ErrorKind,
};

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
            chat_type,
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
            chat_type: Set(chat_type.map(Into::into)),
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

    pub async fn get_stats(&self) -> Result<ChatStats, ErrorKind<Infallible>> {
        use chats::{Column::TgId, Entity};

        #[derive(Default, Debug, FromQueryResult)]
        pub struct CountResult {
            pub count: i64,
        }

        let query = Entity::find().select_only().expr_as(Expr::col(TgId).count(), "count");
        let count = query.into_model::<CountResult>().one(self.conn).await?.unwrap_or_default().count;

        Ok(ChatStats { count })
    }
}
