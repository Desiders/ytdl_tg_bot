use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ColumnTrait as _, ConnectionTrait, EntityTrait, QueryFilter as _};
use std::convert::Infallible;

use crate::{
    database::models::{chat_config_exclude_domains, chat_configs},
    entities::{ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains},
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

    pub async fn get_exclude_domains(&self, tg_id: i64) -> Result<ChatConfigExcludeDomains, ErrorKind<Infallible>> {
        use chat_config_exclude_domains::{Column::TgId, Entity};

        Entity::find()
            .filter(TgId.eq(tg_id))
            .all(self.conn)
            .await
            .map(|rows| ChatConfigExcludeDomains(rows.into_iter().map(|row| row.domain).collect()))
            .map_err(Into::into)
    }

    pub async fn insert_exclude_domain_or_update(
        &self,
        ChatConfigExcludeDomain { tg_id, domain }: ChatConfigExcludeDomain,
    ) -> Result<ChatConfigExcludeDomain, ErrorKind<Infallible>> {
        use chat_config_exclude_domains::{
            ActiveModel,
            Column::{Domain, TgId},
            Entity,
        };

        let model = ActiveModel {
            tg_id: Set(tg_id),
            domain: Set(domain),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::columns([TgId, Domain]).update_columns([Domain]).to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    pub async fn delete_exclude_domain(
        &self,
        ChatConfigExcludeDomain { tg_id, domain }: ChatConfigExcludeDomain,
    ) -> Result<bool, ErrorKind<Infallible>> {
        use chat_config_exclude_domains::{
            Column::{Domain, TgId},
            Entity,
        };

        Entity::delete_many()
            .filter(TgId.eq(tg_id))
            .filter(Domain.eq(domain))
            .exec(self.conn)
            .await
            .map_err(Into::into)
            .map(|res| res.rows_affected > 0)
    }
}
