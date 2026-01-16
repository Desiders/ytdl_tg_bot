use crate::{
    config::RandomCmdConfig,
    database::TxManager,
    entities::{Domains, DownloadedMedia},
    errors::ErrorKind,
    interactors::Interactor,
    value_objects::MediaType,
};

use std::{convert::Infallible, sync::Arc};
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

pub struct GetRandomDownloadedMediaInput<'a> {
    pub limit: u64,
    pub domains: Option<&'a Domains>,
    pub tx_manager: &'a mut TxManager,
}

impl<'a> GetRandomDownloadedMediaInput<'a> {
    pub const fn new(limit: u64, domains: Option<&'a Domains>, tx_manager: &'a mut TxManager) -> Self {
        Self {
            limit,
            domains,
            tx_manager,
        }
    }
}

pub struct GetRandomDownloadedVideo {
    random_cfg: Arc<RandomCmdConfig>,
}

impl GetRandomDownloadedVideo {
    pub const fn new(random_cfg: Arc<RandomCmdConfig>) -> Self {
        Self { random_cfg }
    }
}

impl Interactor<GetRandomDownloadedMediaInput<'_>> for &GetRandomDownloadedVideo {
    type Output = Vec<DownloadedMedia>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%limit, ?domains))]
    async fn execute(
        self,
        GetRandomDownloadedMediaInput {
            limit,
            domains,
            tx_manager,
        }: GetRandomDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;

        let media = dao
            .get_random(
                limit,
                MediaType::Video,
                domains.map_or(&self.random_cfg.domains, |val| val.domains.as_ref()),
            )
            .await?;
        info!(len = media.len(), ?media, "Got random video");

        Ok(media)
    }
}

pub struct GetRandomDownloadedAudio {
    random_cfg: Arc<RandomCmdConfig>,
}

impl GetRandomDownloadedAudio {
    pub const fn new(random_cfg: Arc<RandomCmdConfig>) -> Self {
        Self { random_cfg }
    }
}

impl Interactor<GetRandomDownloadedMediaInput<'_>> for &GetRandomDownloadedAudio {
    type Output = Vec<DownloadedMedia>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%limit, ?domains))]
    async fn execute(
        self,
        GetRandomDownloadedMediaInput {
            limit,
            domains,
            tx_manager,
        }: GetRandomDownloadedMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        tx_manager.begin().await?;

        let dao = tx_manager.downloaded_media_dao()?;

        let media = dao
            .get_random(
                limit,
                MediaType::Audio,
                domains.map_or(&self.random_cfg.domains, |val| val.domains.as_ref()),
            )
            .await?;
        info!(len = media.len(), "Got random audio");

        Ok(media)
    }
}
