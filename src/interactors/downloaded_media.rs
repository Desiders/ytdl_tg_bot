use crate::{
    config::RandomCmdConfig,
    database::TxManager,
    entities::{language::Language, Domains, DownloadedMedia},
    errors::ErrorKind,
    interactors::Interactor,
    value_objects::MediaType,
};

use std::{convert::Infallible, sync::Arc};
use time::OffsetDateTime;
use tracing::{info, instrument};

pub struct AddMediaInput<'a> {
    pub file_id: String,
    pub id: String,
    pub display_id: Option<String>,
    pub domain: Option<String>,
    pub audio_language: Language,
    pub tx_manager: &'a mut TxManager,
}

pub struct AddVideo {}

impl Interactor<AddMediaInput<'_>> for &AddVideo {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%id, ?display_id, ?domain))]
    async fn execute(
        self,
        AddMediaInput {
            file_id,
            id,
            display_id,
            domain,
            audio_language,
            tx_manager,
        }: AddMediaInput<'_>,
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
            audio_language: audio_language.language,
        })
        .await?;
        info!("Downloaded media added");

        tx_manager.commit().await?;
        Ok(())
    }
}

pub struct AddAudio {}

impl Interactor<AddMediaInput<'_>> for &AddAudio {
    type Output = ();
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%id, ?display_id, ?domain))]
    async fn execute(
        self,
        AddMediaInput {
            file_id,
            id,
            display_id,
            domain,
            audio_language,
            tx_manager,
        }: AddMediaInput<'_>,
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
            audio_language: audio_language.language,
        })
        .await?;
        info!("Downloaded media added");

        tx_manager.commit().await?;
        Ok(())
    }
}

pub struct GetRandomMediaInput<'a> {
    pub limit: u64,
    pub domains: Option<&'a Domains>,
    pub tx_manager: &'a mut TxManager,
}

pub struct GetRandomVideo {
    pub random_cfg: Arc<RandomCmdConfig>,
}

impl Interactor<GetRandomMediaInput<'_>> for &GetRandomVideo {
    type Output = Vec<DownloadedMedia>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%limit, ?domains))]
    async fn execute(
        self,
        GetRandomMediaInput {
            limit,
            domains,
            tx_manager,
        }: GetRandomMediaInput<'_>,
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

pub struct GetRandomAudio {
    pub random_cfg: Arc<RandomCmdConfig>,
}

impl Interactor<GetRandomMediaInput<'_>> for &GetRandomAudio {
    type Output = Vec<DownloadedMedia>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%limit, ?domains))]
    async fn execute(
        self,
        GetRandomMediaInput {
            limit,
            domains,
            tx_manager,
        }: GetRandomMediaInput<'_>,
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
