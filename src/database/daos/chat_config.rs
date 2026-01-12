use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait};
use std::convert::Infallible;

use crate::{database::models::chat_configs, entities::ChatConfig, errors::ErrorKind};

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
        ChatConfig {
            tg_id,
            cmd_random_enabled,
            updated_at,
        }: ChatConfig,
    ) -> Result<ChatConfig, ErrorKind<Infallible>> {
        use chat_configs::{
            ActiveModel,
            Column::{CmdRandomEnabled, TgId, UpdatedAt},
            Entity,
        };

        let model = ActiveModel {
            tg_id: Set(tg_id),
            cmd_random_enabled: Set(cmd_random_enabled),
            updated_at: Set(updated_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::column(TgId).update_columns([CmdRandomEnabled, UpdatedAt]).to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
