use async_trait::async_trait;
use sea_orm::{sea_query::OnConflict, ActiveValue::Set, ConnectionTrait, EntityTrait as _};
use std::convert::Infallible;

use crate::{
    database::{interfaces::downloaded_media::DownloadedMediaRepo, models::downloaded_media},
    entities::DownloadedMedia,
    errors::ErrorKind,
};

pub struct SeaOrmDownloadedMediaRepo<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmDownloadedMediaRepo<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> DownloadedMediaRepo for SeaOrmDownloadedMediaRepo<'_, Conn> {
    async fn insert_or_ignore(
        &self,
        DownloadedMedia {
            file_id,
            id,
            domain,
            display_id,
            media_type,
            created_at,
            audio_language,
            crop_start_time,
            crop_end_time,
        }: DownloadedMedia,
    ) -> Result<(), ErrorKind<Infallible>> {
        use downloaded_media::{
            ActiveModel,
            Column::{AudioLanguage, CropEndTime, CropStartTime, Domain, Id, MediaType},
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
            crop_start_time: Set(crop_start_time),
            crop_end_time: Set(crop_end_time),
        };

        Entity::insert(model)
            .on_conflict(
                OnConflict::columns([Id, Domain, MediaType, AudioLanguage, CropStartTime, CropEndTime])
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(self.conn)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }

    async fn insert_or_replace(
        &self,
        DownloadedMedia {
            file_id,
            id,
            domain,
            display_id,
            media_type,
            created_at,
            audio_language,
            crop_start_time,
            crop_end_time,
        }: DownloadedMedia,
    ) -> Result<(), ErrorKind<Infallible>> {
        use downloaded_media::{
            ActiveModel,
            Column::{AudioLanguage, CreatedAt, CropEndTime, CropStartTime, DisplayId, Domain, FileId, Id, MediaType},
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
            crop_start_time: Set(crop_start_time),
            crop_end_time: Set(crop_end_time),
        };

        Entity::insert(model)
            .on_conflict(
                OnConflict::columns([Id, Domain, MediaType, AudioLanguage, CropStartTime, CropEndTime])
                    .update_columns([FileId, DisplayId, CreatedAt])
                    .to_owned(),
            )
            .exec_without_returning(self.conn)
            .await
            .map(|_| ())
            .map_err(Into::into)
    }
}
