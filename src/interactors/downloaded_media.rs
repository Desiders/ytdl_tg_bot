use crate::{
    database::TxManager, entities::DownloadedMedia, errors::database::ErrorKind, interactors::Interactor, value_objects::MediaType,
};

use std::{
    convert::Infallible,
    sync::{Arc, Mutex},
};
use time::OffsetDateTime;
use uuid::{ContextV7, Timestamp, Uuid};

pub struct AddDownloadedMediaInput<'a> {
    pub file_id: Box<str>,
    pub url_or_id: Box<str>,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> AddDownloadedMediaInput<'a> {
    pub const fn new(file_id: Box<str>, url_or_id: Box<str>, tx_manager: &'a mut TxManager) -> Self {
        Self {
            file_id,
            url_or_id,
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

impl Interactor<AddDownloadedMediaInput<'_>> for AddDownloadedVideo {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    async fn execute(
        &mut self,
        AddDownloadedMediaInput {
            file_id,
            url_or_id,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;
        dao.insert_or_ignore(DownloadedMedia {
            id: Uuid::new_v7(Timestamp::now(self.context.as_ref())),
            file_id,
            url_or_id,
            media_type: MediaType::Video,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;

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

impl Interactor<AddDownloadedMediaInput<'_>> for AddDownloadedAudio {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    async fn execute(
        &mut self,
        AddDownloadedMediaInput {
            file_id,
            url_or_id,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;
        dao.insert_or_ignore(DownloadedMedia {
            id: Uuid::new_v7(Timestamp::now(self.context.as_ref())),
            file_id,
            url_or_id,
            media_type: MediaType::Audio,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;

        tx_manager.commit().await?;
        Ok(())
    }
}
