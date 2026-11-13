use async_trait::async_trait;
use sea_orm::{
    sea_query::OnConflict,
    ActiveValue::{NotSet, Set},
    ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
};
use std::{collections::HashMap, convert::Infallible};

use crate::{
    database::{
        interfaces::media::MediaRepo,
        models::{media, media_formats},
    },
    entities::CachedMedia,
    errors::ErrorKind,
};

pub struct SeaOrmMediaRepo<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmMediaRepo<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> MediaRepo for SeaOrmMediaRepo<'_, Conn> {
    async fn replace(&self, mut media: CachedMedia) -> Result<CachedMedia, ErrorKind<Infallible>> {
        use media::Column::{AudioLanguage, Description, MaxFileSize, Title, UpdatedAt, Url};
        use media_formats::Column::{Acodec, Ext, FilesizeApprox, FormatId, Fps, Height, MediaId, Position, Vcodec, Width};
        let row = media::Entity::insert(media::ActiveModel {
            id: Set(media.id),
            url: Set(media.url.clone()),
            audio_language: Set(media.audio_language.clone()),
            max_file_size: Set(media.max_file_size),
            title: Set(media.title.clone()),
            description: Set(media.description.clone()),
            updated_at: Set(media.updated_at),
        })
        .on_conflict(
            OnConflict::columns([Url, AudioLanguage, MaxFileSize])
                .update_columns([Title, Description, UpdatedAt])
                .to_owned(),
        )
        .exec_with_returning(self.conn)
        .await?;
        media.id = row.id;
        let formats = media
            .formats
            .iter()
            .enumerate()
            .map(|(position, entry)| {
                let format = &entry.format;
                media_formats::ActiveModel {
                    id: NotSet,
                    media_id: Set(media.id),
                    format_id: Set(format.format_id.clone()),
                    position: Set(i32::try_from(position).unwrap()),
                    ext: Set(format.ext.clone()),
                    width: Set(format.width.unwrap()),
                    height: Set(format.height.unwrap()),
                    fps: Set(format.fps),
                    vcodec: Set(format.vcodec.clone()),
                    acodec: Set(format.acodec.clone()),
                    filesize_approx: Set(format.filesize_approx.and_then(|size| i64::try_from(size).ok())),
                }
            })
            .collect::<Vec<_>>();
        let saved = media_formats::Entity::insert_many(formats)
            .on_conflict(
                OnConflict::columns([MediaId, FormatId])
                    .update_columns([Position, Ext, Width, Height, Fps, Vcodec, Acodec, FilesizeApprox])
                    .to_owned(),
            )
            .exec_with_returning(self.conn)
            .await?;
        let ids: HashMap<_, _> = saved.into_iter().map(|format| (format.format_id, format.id)).collect();
        for format in &mut media.formats {
            format.id = ids[&format.format.format_id];
        }
        media_formats::Entity::delete_many()
            .filter(MediaId.eq(media.id))
            .filter(FormatId.is_not_in(ids.keys().cloned()))
            .exec(self.conn)
            .await?;
        Ok(media)
    }
}
