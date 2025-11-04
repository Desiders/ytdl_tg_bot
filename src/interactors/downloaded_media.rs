use crate::{
    database::TxManager, entities::DownloadedMedia, errors::database::ErrorKind, interactors::Interactor, value_objects::MediaType,
};

use std::convert::Infallible;
use time::OffsetDateTime;
use tracing::{event, instrument, Level};

pub struct AddDownloadedMediaInput<'a> {
    pub file_id: String,
    pub id: String,
    pub domain: Option<String>,
    pub chat_tg_id: i64,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> AddDownloadedMediaInput<'a> {
    pub const fn new(file_id: String, id: String, domain: Option<String>, chat_tg_id: i64, tx_manager: &'a mut TxManager) -> Self {
        Self {
            file_id,
            id,
            domain,
            chat_tg_id,
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
            domain,
            chat_tg_id,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = match domain {
            Some(domain) => match domain.strip_prefix("www.") {
                Some(domain) => Some(domain.to_owned()),
                None => Some(domain),
            },
            None => None,
        };

        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;
        dao.insert_or_ignore(DownloadedMedia {
            file_id,
            id,
            domain: normalized_domain,
            media_type: MediaType::Video,
            chat_tg_id,
            created_at: OffsetDateTime::now_utc(),
        })
        .await?;
        event!(Level::INFO, "Downloaded media added");

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
            domain,
            chat_tg_id,
            tx_manager,
        }: AddDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = match domain {
            Some(domain) => match domain.strip_prefix("www.") {
                Some(domain) => Some(domain.to_owned()),
                None => Some(domain),
            },
            None => None,
        };

        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;

        dao.insert_or_ignore(DownloadedMedia {
            file_id,
            id,
            domain: normalized_domain,
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
