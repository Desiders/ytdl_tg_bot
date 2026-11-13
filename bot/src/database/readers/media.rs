use async_trait::async_trait;
use sea_orm::{ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _, QueryOrder as _};
use std::convert::Infallible;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{
    database::{
        interfaces::media::MediaReader,
        models::{media, media_formats},
    },
    entities::{CachedMedia, CachedMediaFormat, MediaFormat},
    errors::ErrorKind,
};

pub struct SeaOrmMediaReader<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmMediaReader<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

impl<Conn: ConnectionTrait> SeaOrmMediaReader<'_, Conn> {
    async fn load(&self, row: Option<media::Model>) -> Result<Option<CachedMedia>, ErrorKind<Infallible>> {
        let Some(row) = row else {
            return Ok(None);
        };
        let formats = media_formats::Entity::find()
            .filter(media_formats::Column::MediaId.eq(row.id))
            .order_by_asc(media_formats::Column::Position)
            .all(self.conn)
            .await?;
        if formats.is_empty() {
            return Ok(None);
        }
        Ok(Some(CachedMedia {
            id: row.id,
            url: row.url,
            audio_language: row.audio_language,
            max_file_size: row.max_file_size,
            title: row.title,
            description: row.description,
            updated_at: row.updated_at,
            formats: formats
                .into_iter()
                .map(|format| CachedMediaFormat {
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
impl<Conn: ConnectionTrait> MediaReader for SeaOrmMediaReader<'_, Conn> {
    async fn get_by_url(
        &self,
        url: &str,
        audio_language: &str,
        max_file_size: i64,
        since: OffsetDateTime,
    ) -> Result<Option<CachedMedia>, ErrorKind<Infallible>> {
        let row = media::Entity::find()
            .filter(media::Column::Url.eq(url))
            .filter(media::Column::AudioLanguage.eq(audio_language))
            .filter(media::Column::MaxFileSize.eq(max_file_size))
            .filter(media::Column::UpdatedAt.gte(since))
            .one(self.conn)
            .await?;
        self.load(row).await
    }

    async fn get(&self, id: Uuid) -> Result<Option<CachedMedia>, ErrorKind<Infallible>> {
        let row = media::Entity::find_by_id(id).one(self.conn).await?;
        self.load(row).await
    }
}
