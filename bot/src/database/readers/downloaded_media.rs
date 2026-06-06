use async_trait::async_trait;
use sea_orm::{
    prelude::Expr, ColumnTrait as _, ConnectionTrait, EntityTrait as _, ExprTrait as _, FromQueryResult, QueryFilter as _, QueryOrder as _,
    QuerySelect as _,
};
use std::convert::Infallible;
use time::{Duration, OffsetDateTime};

use crate::{
    database::{
        interfaces::downloaded_media::DownloadedMediaReader,
        models::{downloaded_media, sea_orm_active_enums},
    },
    entities::{DownloadedMedia, DownloadedMediaByDomainCount, DownloadedMediaCount, DownloadedMediaStats},
    errors::ErrorKind,
    value_objects::MediaType,
};

pub struct SeaOrmDownloadedMediaReader<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmDownloadedMediaReader<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> DownloadedMediaReader for SeaOrmDownloadedMediaReader<'_, Conn> {
    async fn get(
        &self,
        search: &str,
        domain: Option<&str>,
        audio_language: Option<&str>,
        media_type: MediaType,
        crop_start_time: Option<i32>,
        crop_end_time: Option<i32>,
    ) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{AudioLanguage, CropEndTime, CropStartTime, DisplayId, Id, MediaType},
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
        if let Some(time) = crop_start_time {
            query = query.filter(CropStartTime.eq(time));
        } else {
            query = query.filter(CropStartTime.is_null());
        }
        if let Some(time) = crop_end_time {
            query = query.filter(CropEndTime.eq(time));
        } else {
            query = query.filter(CropEndTime.is_null());
        }

        Ok(query.one(self.conn).await?.map(Into::into))
    }

    async fn get_random(
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

    async fn get_stats(&self, top_domains_limit: u64) -> Result<DownloadedMediaStats, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{CreatedAt, Domain, FileId},
            Entity,
        };

        #[derive(Default, Debug, FromQueryResult)]
        struct CountResult {
            count: i64,
        }

        #[derive(Debug, FromQueryResult)]
        struct DomainCountResult {
            domain: String,
            count: i64,
        }

        async fn count_by_period<Conn>(conn: &Conn, since: Option<OffsetDateTime>) -> Result<DownloadedMediaCount, ErrorKind<Infallible>>
        where
            Conn: ConnectionTrait,
        {
            let mut query = Entity::find().select_only().expr_as(Expr::col(FileId).count(), "count");
            if let Some(since) = since {
                query = query.filter(Expr::col(CreatedAt).gte(since));
            }
            let count = query.into_model::<CountResult>().one(conn).await?.unwrap_or_default().count;
            Ok(DownloadedMediaCount { count })
        }

        let now = OffsetDateTime::now_utc();

        let count_total = count_by_period(self.conn, None).await?;
        let count_last_day = count_by_period(self.conn, Some(now - Duration::days(1))).await?;
        let count_last_week = count_by_period(self.conn, Some(now - Duration::days(7))).await?;
        let count_last_month = count_by_period(self.conn, Some(now - Duration::days(30))).await?;
        let top_domains = Entity::find()
            .select_only()
            .column(Domain)
            .expr_as(Expr::col(Domain).count(), "count")
            .filter(Expr::col(Domain).is_not_null())
            .group_by(Domain)
            .order_by_desc(Expr::col("count"))
            .limit(Some(top_domains_limit))
            .into_model::<DomainCountResult>()
            .all(self.conn)
            .await?
            .into_iter()
            .map(|val| DownloadedMediaByDomainCount {
                domain: val.domain,
                count: val.count,
            })
            .collect();

        Ok(DownloadedMediaStats {
            last_day: count_last_day,
            last_week: count_last_week,
            last_month: count_last_month,
            total: count_total,
            top_domains,
        })
    }
}
