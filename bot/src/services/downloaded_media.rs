use crate::{
    config::RandomCmdConfig,
    database::TxManager,
    entities::{language::Language, ChatStats, Domains, DownloadedMedia, DownloadedMediaStats, Sections},
    errors::ErrorKind,
    interactors::Interactor,
    value_objects::MediaType,
};

use std::{convert::Infallible, sync::Arc};
use time::OffsetDateTime;
use tracing::{info, instrument};

pub struct AddMediaInput {
    pub file_id: String,
    pub id: String,
    pub display_id: Option<String>,
    pub domain: Option<String>,
    pub audio_language: Language,
    pub sections: Option<Sections>,
    pub overwrite_cache: bool,
}

pub struct AddVideo {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl AddVideo {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<AddMediaInput> for &AddVideo {
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
            sections,
            overwrite_cache,
        }: AddMediaInput,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = domain.map(|domain| domain.trim_start_matches("www.").to_owned());
        let media = DownloadedMedia {
            file_id,
            id,
            display_id,
            domain: normalized_domain,
            media_type: MediaType::Video,
            created_at: OffsetDateTime::now_utc(),
            audio_language: audio_language.language,
            crop_start_time: sections.as_ref().and_then(|val| val.start),
            crop_end_time: sections.as_ref().and_then(|val| val.end),
        };
        add_media(&**self.tx_manager, media, overwrite_cache).await
    }
}

pub struct AddAudio {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl AddAudio {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<AddMediaInput> for &AddAudio {
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
            sections,
            overwrite_cache,
        }: AddMediaInput,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = domain.map(|domain| domain.trim_start_matches("www.").to_owned());
        let media = DownloadedMedia {
            file_id,
            id,
            display_id,
            domain: normalized_domain,
            media_type: MediaType::Audio,
            created_at: OffsetDateTime::now_utc(),
            audio_language: audio_language.language,
            crop_start_time: sections.as_ref().and_then(|val| val.start),
            crop_end_time: sections.as_ref().and_then(|val| val.end),
        };
        add_media(&**self.tx_manager, media, overwrite_cache).await
    }
}

pub struct AddPhoto {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl AddPhoto {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

impl Interactor<AddMediaInput> for &AddPhoto {
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
            sections,
            overwrite_cache,
        }: AddMediaInput,
    ) -> Result<Self::Output, Self::Err> {
        let normalized_domain = domain.map(|domain| domain.trim_start_matches("www.").to_owned());
        let media = DownloadedMedia {
            file_id,
            id,
            display_id,
            domain: normalized_domain,
            media_type: MediaType::Photo,
            created_at: OffsetDateTime::now_utc(),
            audio_language: audio_language.language,
            crop_start_time: sections.as_ref().and_then(|val| val.start),
            crop_end_time: sections.as_ref().and_then(|val| val.end),
        };
        add_media(&**self.tx_manager, media, overwrite_cache).await
    }
}

async fn add_media(tx_manager: &dyn TxManager, media: DownloadedMedia, overwrite_cache: bool) -> Result<(), ErrorKind<Infallible>> {
    let tx = tx_manager.begin().await?;
    let outcome = {
        let repo = tx.downloaded_media_repo();
        if overwrite_cache {
            repo.insert_or_replace(media).await
        } else {
            repo.insert_or_ignore(media).await
        }
    };
    if let Err(err) = outcome {
        let _ = tx.rollback().await;
        return Err(err);
    }
    info!("Downloaded media added");

    tx.commit().await?;
    Ok(())
}

pub struct GetRandomMediaInput<'a> {
    pub limit: u64,
    pub domains: Option<&'a Domains>,
}

pub struct GetRandomVideo {
    cfg: Arc<RandomCmdConfig>,
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetRandomVideo {
    #[must_use]
    pub const fn new(cfg: Arc<RandomCmdConfig>, tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { cfg, tx_manager }
    }
}

impl Interactor<GetRandomMediaInput<'_>> for &GetRandomVideo {
    type Output = Vec<DownloadedMedia>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%limit, ?domains))]
    async fn execute(self, GetRandomMediaInput { limit, domains }: GetRandomMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        let media = self
            .tx_manager
            .downloaded_media_reader()
            .get_random(
                limit,
                MediaType::Video,
                domains.map_or(&self.cfg.domains, |val| val.domains.as_ref()),
            )
            .await?;
        info!(len = media.len(), ?media, "Got random video");

        Ok(media)
    }
}

pub struct GetRandomAudio {
    cfg: Arc<RandomCmdConfig>,
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetRandomAudio {
    #[must_use]
    pub const fn new(cfg: Arc<RandomCmdConfig>, tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { cfg, tx_manager }
    }
}

impl Interactor<GetRandomMediaInput<'_>> for &GetRandomAudio {
    type Output = Vec<DownloadedMedia>;
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all, fields(%limit, ?domains))]
    async fn execute(self, GetRandomMediaInput { limit, domains }: GetRandomMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        let media = self
            .tx_manager
            .downloaded_media_reader()
            .get_random(
                limit,
                MediaType::Audio,
                domains.map_or(&self.cfg.domains, |val| val.domains.as_ref()),
            )
            .await?;
        info!(len = media.len(), "Got random audio");

        Ok(media)
    }
}

pub struct GetStats {
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetStats {
    #[must_use]
    pub const fn new(tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self { tx_manager }
    }
}

pub struct GetStatsInput {
    pub top_domains_limit: u64,
}

impl Interactor<GetStatsInput> for &GetStats {
    type Output = (DownloadedMediaStats, ChatStats);
    type Err = ErrorKind<Infallible>;

    #[instrument(skip_all)]
    async fn execute(self, GetStatsInput { top_domains_limit }: GetStatsInput) -> Result<Self::Output, Self::Err> {
        let media_stats = self.tx_manager.downloaded_media_reader().get_stats(top_domains_limit).await?;
        info!(?media_stats, "Got media stats");

        let chat_stats = self.tx_manager.chat_reader().get_stats().await?;
        info!(?chat_stats, "Got chat stats");

        Ok((media_stats, chat_stats))
    }
}
