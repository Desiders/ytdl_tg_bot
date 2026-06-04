use async_trait::async_trait;
use sea_orm::{prelude::Expr, ConnectionTrait, EntityTrait as _, ExprTrait as _, FromQueryResult, QuerySelect as _};
use std::convert::Infallible;

use crate::{
    database::{
        interfaces::chat::ChatReader,
        models::{chats, sea_orm_active_enums},
    },
    entities::{ChatStats, ChatTypeCount},
    errors::ErrorKind,
};

pub struct SeaOrmChatReader<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmChatReader<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> ChatReader for SeaOrmChatReader<'_, Conn> {
    async fn get_stats(&self) -> Result<ChatStats, ErrorKind<Infallible>> {
        use chats::{
            Column::{ChatType, TgId},
            Entity,
        };

        #[derive(Default, Debug, FromQueryResult)]
        struct CountResult {
            count: i64,
        }

        #[derive(Debug, FromQueryResult)]
        struct ChatTypeCountResult {
            chat_type: Option<sea_orm_active_enums::ChatType>,
            count: i64,
        }

        let query = Entity::find().select_only().expr_as(Expr::col(TgId).count(), "count");
        let count = query.into_model::<CountResult>().one(self.conn).await?.unwrap_or_default().count;

        let by_type = Entity::find()
            .select_only()
            .column(ChatType)
            .expr_as(Expr::col(TgId).count(), "count")
            .group_by(ChatType)
            .into_model::<ChatTypeCountResult>()
            .all(self.conn)
            .await?
            .into_iter()
            .map(|row| ChatTypeCount {
                kind: row.chat_type.map(Into::into),
                count: row.count,
            })
            .collect();

        Ok(ChatStats { count, by_type })
    }
}
