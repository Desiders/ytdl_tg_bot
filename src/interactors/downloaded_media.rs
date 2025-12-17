use crate::{database::TxManager, entities::DownloadedMedia, errors::ErrorKind, interactors::Interactor, value_objects::MediaType};

use std::convert::Infallible;
use time::OffsetDateTime;
use tracing::{info, instrument};

pub struct AddDownloadedMediaInput<'a> {
    pub file_id: String,
    pub id: String,
    pub display_id: Option<String>,
    pub domain: Option<String>,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> AddDownloadedMediaInput<'a> {
    pub const fn new(
        file_id: String,
        id: String,
        display_id: Option<String>,
        domain: Option<String>,
        tx_manager: &'a mut TxManager,
    ) -> Self {
        Self {
            file_id,
            id,
            display_id,
            domain,
            tx_manager,
        }
    }
}

pub struct AddDownloadedVideo {}

impl AddDownloadedVideo {
    pub const fn new() -> Self {
        Self {}
    }
}

impl Interactor<AddDownloadedMediaInput<'_>> for &AddDownloadedVideo {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%id, ?domain))]
    async fn execute(
        self,
        AddDownloadedMediaInput {
            file_id,
            id,
            display_id,
            domain,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = domain.map(|domain| domain.trim_start_matches("www.").to_owned());

        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;
        dao.insert_or_ignore(DownloadedMedia {
            file_id,
            id,
            display_id,
            domain: normalized_domain,
            media_type: MediaType::Video,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;
        info!("Downloaded media added");

        tx_manager.commit().await?;
        Ok(())
    }
}

pub struct AddDownloadedAudio {}

impl AddDownloadedAudio {
    pub const fn new() -> Self {
        Self {}
    }
}

impl Interactor<AddDownloadedMediaInput<'_>> for &AddDownloadedAudio {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%id, ?domain))]
    async fn execute(
        self,
        AddDownloadedMediaInput {
            file_id,
            id,
            display_id,
            domain,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = domain.map(|domain| domain.trim_start_matches("www.").to_owned());

        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;

        dao.insert_or_ignore(DownloadedMedia {
            file_id,
            id,
            display_id,
            domain: normalized_domain,
            media_type: MediaType::Audio,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;
        info!("Downloaded media added");

        tx_manager.commit().await?;
        Ok(())
    }
}
