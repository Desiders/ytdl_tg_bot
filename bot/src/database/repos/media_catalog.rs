use async_trait::async_trait;
use sea_orm::{
    sea_query::OnConflict,
    ActiveValue::{NotSet, Set},
    ColumnTrait as _, ConnectionTrait, EntityTrait as _, QueryFilter as _,
};
use std::{collections::HashMap, convert::Infallible};

use crate::{
    database::{
        interfaces::media_catalog::MediaCatalogRepo,
        models::{media_catalogs, media_formats},
    },
    entities::MediaCatalog,
    errors::ErrorKind,
};

pub struct SeaOrmMediaCatalogRepo<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmMediaCatalogRepo<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> MediaCatalogRepo for SeaOrmMediaCatalogRepo<'_, Conn> {
    async fn replace(&self, mut catalog: MediaCatalog) -> Result<MediaCatalog, ErrorKind<Infallible>> {
        use media_catalogs::Column::{AudioLanguage, Description, Duration, MaxFileSize, MediaId, Title, UpdatedAt, Url, WebpageUrl};
        use media_formats::Column::{Acodec, CatalogId, Ext, FilesizeApprox, FormatId, Fps, Height, Position, Vcodec, Width};
        let row = media_catalogs::Entity::insert(media_catalogs::ActiveModel {
            id: Set(catalog.id),
            url: Set(catalog.url.clone()),
            audio_language: Set(catalog.audio_language.clone()),
            max_file_size: Set(catalog.max_file_size),
            media_id: Set(catalog.media_id.clone()),
            webpage_url: Set(catalog.webpage_url.clone()),
            title: Set(catalog.title.clone()),
            description: Set(catalog.description.clone()),
            duration: Set(catalog.duration),
            updated_at: Set(catalog.updated_at),
        })
        .on_conflict(
            OnConflict::columns([Url, AudioLanguage, MaxFileSize])
                .update_columns([MediaId, WebpageUrl, Title, Description, Duration, UpdatedAt])
                .to_owned(),
        )
        .exec_with_returning(self.conn)
        .await?;
        catalog.id = row.id;
        let formats = catalog
            .formats
            .iter()
            .enumerate()
            .map(|(position, entry)| {
                let format = &entry.format;
                media_formats::ActiveModel {
                    id: NotSet,
                    catalog_id: Set(catalog.id),
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
                OnConflict::columns([CatalogId, FormatId])
                    .update_columns([Position, Ext, Width, Height, Fps, Vcodec, Acodec, FilesizeApprox])
                    .to_owned(),
            )
            .exec_with_returning(self.conn)
            .await?;
        let ids: HashMap<_, _> = saved.into_iter().map(|format| (format.format_id, format.id)).collect();
        for format in &mut catalog.formats {
            format.id = ids[&format.format.format_id];
        }
        media_formats::Entity::delete_many()
            .filter(CatalogId.eq(catalog.id))
            .filter(FormatId.is_not_in(ids.keys().cloned()))
            .exec(self.conn)
            .await?;
        Ok(catalog)
    }
}
