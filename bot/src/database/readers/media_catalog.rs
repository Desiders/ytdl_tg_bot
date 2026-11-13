use async_trait::async_trait;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _, QueryOrder as _};
use std::convert::Infallible;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    database::{
        interfaces::media_catalog::MediaCatalogReader,
        models::{media_catalogs, media_formats},
    },
    entities::{MediaCatalog, MediaCatalogFormat, MediaFormat},
    errors::ErrorKind,
};

pub struct SeaOrmMediaCatalogReader<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmMediaCatalogReader<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

impl<Conn: ConnectionTrait> SeaOrmMediaCatalogReader<'_, Conn> {
    async fn load(&self, row: Option<media_catalogs::Model>) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>> {
        let Some(row) = row else {
            return Ok(None);
        };
        let formats = media_formats::Entity::find()
            .filter(media_formats::Column::CatalogId.eq(row.id))
            .order_by_asc(media_formats::Column::Position)
            .all(self.conn)
            .await?;
        if formats.is_empty() {
            return Ok(None);
        }
        Ok(Some(MediaCatalog {
            id: row.id,
            url: row.url,
            audio_language: row.audio_language,
            max_file_size: row.max_file_size,
            media_id: row.media_id,
            webpage_url: row.webpage_url,
            title: row.title,
            description: row.description,
            duration: row.duration,
            updated_at: row.updated_at,
            formats: formats
                .into_iter()
                .map(|format| MediaCatalogFormat {
                    id: format.id,
                    format: MediaFormat {
                        format_id: format.format_id,
                        format_note: None,
                        ext: format.ext,
                        width: Some(format.width),
                        height: Some(format.height),
                        fps: format.fps,
                        vcodec: format.vcodec,
                        acodec: format.acodec,
                        aspect_ratio: None,
                        filesize_approx: format.filesize_approx.and_then(|size| u64::try_from(size).ok()),
                    },
                })
                .collect(),
        }))
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> MediaCatalogReader for SeaOrmMediaCatalogReader<'_, Conn> {
    async fn get_by_url(
        &self,
        url: &str,
        audio_language: &str,
        max_file_size: i64,
        since: OffsetDateTime,
    ) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>> {
        let row = media_catalogs::Entity::find()
            .filter(media_catalogs::Column::Url.eq(url))
            .filter(media_catalogs::Column::AudioLanguage.eq(audio_language))
            .filter(media_catalogs::Column::MaxFileSize.eq(max_file_size))
            .filter(media_catalogs::Column::UpdatedAt.gte(since))
            .one(self.conn)
            .await?;
        self.load(row).await
    }

    async fn get(&self, id: Uuid) -> Result<Option<MediaCatalog>, ErrorKind<Infallible>> {
        let row = media_catalogs::Entity::find_by_id(id).one(self.conn).await?;
        self.load(row).await
    }
}
