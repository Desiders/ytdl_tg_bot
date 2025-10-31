use crate::{
    database::TxManager, entities::DownloadedMedia, errors::database::ErrorKind, interactors::Interactor, value_objects::MediaType,
};

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use time::OffsetDateTime;
use tracing::{event, Level};
use uuid::ContextV7;

pub struct AddDownloadedMediaInput<'a> {
    pub file_id: String,
    pub url_or_id: String,
    pub index_in_playlist: i16,
    pub chat_tg_id: i64,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> AddDownloadedMediaInput<'a> {
    pub const fn new(file_id: String, url_or_id: String, index_in_playlist: i16, chat_tg_id: i64, tx_manager: &'a mut TxManager) -> Self {
        Self {
            file_id,
            url_or_id,
            index_in_playlist,
            chat_tg_id,
            tx_manager,
        }
    }
}

pub struct AddDownloadedVideo {
    context: Arc<Mutex<ContextV7>>,
}

impl AddDownloadedVideo {
    pub const fn new(context: Arc<Mutex<ContextV7>>) -> Self {
        Self { context }
    }
}

impl Interactor<AddDownloadedMediaInput<'_>> for &AddDownloadedVideo {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    async fn execute(
        self,
        AddDownloadedMediaInput {
            file_id,
            url_or_id,
            index_in_playlist,
            chat_tg_id,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;
        dao.insert_or_ignore(DownloadedMedia {
            file_id,
            url_or_id,
            media_type: MediaType::Video,
            index_in_playlist,
            chat_tg_id,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;
        event!(Level::INFO, "Downloaded media added");

        tx_manager.commit().await?;
        Ok(())
    }
}

pub struct AddDownloadedAudio {
    context: Arc<Mutex<ContextV7>>,
}

impl AddDownloadedAudio {
    pub const fn new(context: Arc<Mutex<ContextV7>>) -> Self {
        Self { context }
    }
}

impl Interactor<AddDownloadedMediaInput<'_>> for &AddDownloadedAudio {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    async fn execute(
        self,
        AddDownloadedMediaInput {
            file_id,
            url_or_id,
            index_in_playlist,
            chat_tg_id,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;
        dao.insert_or_ignore(DownloadedMedia {
            file_id,
            url_or_id,
            index_in_playlist,
            media_type: MediaType::Audio,
            chat_tg_id,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;
        event!(Level::INFO, "Downloaded media added");

        tx_manager.commit().await?;
        Ok(())
    }
}
