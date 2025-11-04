use sea_orm::{prelude::Expr, sea_query::OnConflict, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait as _, QueryFilter as _};
use std::convert::Infallible;

use crate::{
    database::models::{downloaded_media, sea_orm_active_enums},
    entities::DownloadedMedia,
    errors::database::ErrorKind,
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
            media_type,
            chat_tg_id,
            created_at,
        }: DownloadedMedia,
    ) -> Result<(), ErrorKind<Infallible>> {
        use downloaded_media::{
            ActiveModel,
            Column::{Domain, Id, MediaType},
            Entity,
        };

        let normalized_domain = domain.map(|domain| domain.strip_prefix("www.").map(ToOwned::to_owned)).flatten();

        let model = ActiveModel {
            file_id: Set(file_id.into()),
            id: Set(id.into()),
            domain: Set(normalized_domain.into()),
            media_type: Set(media_type.into()),
            chat_tg_id: Set(chat_tg_id.into()),
            created_at: Set(created_at),
        };

        Entity::insert(model)
            .on_conflict(OnConflict::columns([Id, Domain, MediaType]).do_nothing().to_owned())
            .exec_without_returning(self.conn)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    pub async fn get_by_id_or_url_and_domain(
        &self,
        id_or_url: &str,
        domain: Option<&str>,
        media_type: MediaType,
    ) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{Domain, MediaType},
            Entity,
        };

        let mut query = Entity::find()
            .filter(MediaType.eq(sea_orm_active_enums::MediaType::from(media_type)))
            .filter(Expr::cust_with_values("$1 LIKE '%' || id::text || '%'", [id_or_url]));

        if let Some(domain) = domain {
            let normalized_domain = domain.strip_prefix("www.").unwrap_or(domain);
            query = query.filter(Domain.eq(normalized_domain));
        }

        Ok(query.one(self.conn).await?.map(Into::into))
    }
}
