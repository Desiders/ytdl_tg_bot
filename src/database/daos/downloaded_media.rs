use sea_orm::{
    prelude::Expr, sea_query::OnConflict, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait as _, QueryFilter as _, QueryOrder,
    QuerySelect,
};
use std::convert::Infallible;

use crate::{
    database::models::{downloaded_media, sea_orm_active_enums},
    entities::DownloadedMedia,
    errors::ErrorKind,
    value_objects::MediaType,
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
    pub async fn insert_or_ignore(
        &self,
        DownloadedMedia {
            file_id,
            id,
            domain,
            display_id,
            media_type,
            created_at,
            audio_language,
        }: DownloadedMedia,
    ) -> Result<(), ErrorKind<Infallible>> {
        use downloaded_media::{
            ActiveModel,
            Column::{AudioLanguage, Domain, Id, MediaType},
            Entity,
        };

        let model = ActiveModel {
            file_id: Set(file_id),
            id: Set(id),
            display_id: Set(display_id),
            domain: Set(domain),
            media_type: Set(media_type.into()),
            created_at: Set(created_at),
            audio_language: Set(audio_language),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::columns([Id, Domain, MediaType, AudioLanguage]).do_nothing().to_owned())
            .exec_without_returning(self.conn)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn get(
        &self,
        search: &str,
        domain: Option<&str>,
        audio_language: Option<&str>,
        media_type: MediaType,
    ) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{AudioLanguage, DisplayId, Id, MediaType},
            Entity,
        };

        let mut query = Entity::find()
            .filter(MediaType.eq(sea_orm_active_enums::MediaType::from(media_type)))
            .filter(
                // if `search` is ID
                Expr::col(Id)
                    .eq(search)
                    .or(Expr::col(DisplayId).eq(search))
                    // if `search` is URL
                    .or(Expr::cust_with_values("$1 ~ ('(^|[/?&=])' || id::text || '([&?/]|$)')", [search]))
                    .or(Expr::cust_with_values(
                        "$1 ~ ('(^|[/?&=])' || display_id::text || '([&?/]|$)')",
                        [search],
                    )),
            );
        if let Some(lang) = audio_language {
            query = query.filter(AudioLanguage.eq(lang));
        }
        if let Some(domain) = domain {
            query = query.filter(Expr::cust_with_values("$1 ~* ('(^|\\.)' || domain || '$')", [domain]));
        }

        Ok(query.one(self.conn).await?.map(Into::into))
    }

    pub async fn get_random(
        &self,
        limit: u64,
        media_type: MediaType,
        domains: &[String],
    ) -> Result<Vec<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{Domain, MediaType},
            Entity,
        };

        Ok(Entity::find()
            .filter(MediaType.eq(sea_orm_active_enums::MediaType::from(media_type)))
            .filter(Domain.is_in(domains))
            .order_by_desc(Expr::cust("RANDOM()"))
            .limit(limit)
            .all(self.conn)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }
}
