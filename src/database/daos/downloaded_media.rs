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
            id,
            tg_id,
            url,
            id_in_url,
            media_type,
            created_at,
        }: DownloadedMedia,
    ) -> Result<DownloadedMedia, ErrorKind<Infallible>> {
        use downloaded_media::{
            ActiveModel,
            Column::{MediaType, TgId},
            Entity,
        };

        let model = ActiveModel {
            id: Set(id),
            tg_id: Set(tg_id),
            url: Set(url.into()),
            id_in_url: Set(id_in_url.map(Into::into)),
            media_type: Set(media_type.into()),
            created_at: Set(created_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::columns([TgId, MediaType]).do_nothing().to_owned())
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }

    pub async fn get_by_id_or_url(
        &self,
        id_in_url: Option<Box<str>>,
        url: Box<str>,
    ) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{IdInUrl, Url},
            Entity,
        };

        Entity::find()
            .filter(IdInUrl.eq(id_in_url.as_deref()).or(Url.eq(url.as_ref())))
            .one(self.conn)
            .await
            .map(|val| val.map(Into::into))
            .map_err(Into::into)
    }
}
