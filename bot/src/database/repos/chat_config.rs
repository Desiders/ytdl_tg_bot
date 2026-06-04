use async_trait::async_trait;
use sea_orm::{
    sea_query::OnConflict,
    ActiveValue::{NotSet, Set, Unchanged},
    ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
};
use std::convert::Infallible;

use crate::{
    database::{
        interfaces::chat_config::ChatConfigRepo,
        models::{chat_config_exclude_domains, chat_configs},
    },
    entities::{ChatConfig, ChatConfigExcludeDomain, ChatConfigUpdate},
    errors::ErrorKind,
};

pub struct SeaOrmChatConfigRepo<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmChatConfigRepo<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> ChatConfigRepo for SeaOrmChatConfigRepo<'_, Conn> {
    async fn insert_or_update(
        &self,
        ChatConfig {
            tg_id,
            cmd_random_enabled,
            link_is_visible,
            language,
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
            link_is_visible: Unchanged(link_is_visible),
            language: Unchanged(language),
            updated_at: Set(updated_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::column(TgId).update_columns([CmdRandomEnabled, UpdatedAt]).to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    async fn update(
        &self,
        ChatConfigUpdate {
            tg_id,
            cmd_random_enabled,
            link_is_visible,
            language,
            updated_at,
        }: ChatConfigUpdate,
    ) -> Result<ChatConfig, ErrorKind<Infallible>> {
        use chat_configs::{ActiveModel, Entity};

        let model = ActiveModel {
            tg_id: Set(tg_id),
            cmd_random_enabled: cmd_random_enabled.map_or(NotSet, Set),
            link_is_visible: link_is_visible.map_or(NotSet, Set),
            language: language.map_or(NotSet, Set),
            updated_at: Set(updated_at),
        };

        Entity::update(model).exec(self.conn).await.map(Into::into).map_err(Into::into)
    }

    async fn insert_exclude_domain_or_update(
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

    async fn delete_exclude_domain(
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
