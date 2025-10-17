use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use std::convert::Infallible;

use crate::{database::models::downloaded_media, entities::DownloadedMedia, errors::database::ErrorKind};

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
    pub async fn insert_or_ignore(
        &self,
        DownloadedMedia {
            file_id,
            url_or_id,
            media_type,
            chat_tg_id,
            created_at,
        }: DownloadedMedia,
    ) -> Result<DownloadedMedia, ErrorKind<Infallible>> {
        use downloaded_media::{
            ActiveModel,
            Column::{MediaType, UrlOrId},
            Entity,
        };

        let model = ActiveModel {
            file_id: Set(file_id.into()),
            url_or_id: Set(url_or_id.into()),
            media_type: Set(media_type.into()),
            chat_tg_id: Set(chat_tg_id.into()),
            created_at: Set(created_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::columns([UrlOrId, MediaType]).do_nothing().to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    pub async fn get_by_url_or_id(&self, url_or_id: Box<str>) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{Column::UrlOrId, Entity};

        Entity::find()
            .filter(UrlOrId.eq(url_or_id.as_ref()))
            .one(self.conn)
            .await
            .map(|val| val.map(Into::into))
            .map_err(Into::into)
    }
}
