use std::{collections::HashSet, convert::Infallible, sync::Arc};

use reqwest::Client;
use tonic::Code;
use tracing::{debug, error, info, instrument, warn};
use url::Url;
use ytdl_tg_bot_proto::downloader::{downloader_client::DownloaderClient, MediaInfoRequest};

use crate::{
    config::{TrackingParamsConfig, YtToolkitConfig},
    database::TxManager,
    entities::{
        language::Language, yt_toolkit::BasicInfo, DownloadedMedia, Media, MediaFormat, MediaInPlaylist, Playlist, Range,
        RawMediaWithFormat, Sections,
    },
    errors::ErrorKind,
    interactors::Interactor,
    services::{
        node_router::{authenticated_request, NodeRouter},
        yt_toolkit::{get_video_info, search_video, GetVideoInfoErrorKind, SearchVideoErrorKind},
    },
    value_objects::MediaType,
};

const MAX_DECODING_MESSAGE_SIZE: usize = 30 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum GetInfoErrorKind {
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error("Invalid node response: {0}")]
    InvalidResponse(Box<str>),
    #[error("No download node available")]
    NodeUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum GetMediaByURLErrorKind {
    #[error(transparent)]
    GetInfo(#[from] GetInfoErrorKind),
    #[error(transparent)]
    Database(#[from] ErrorKind<Infallible>),
    #[error("No download node available")]
    NodeUnavailable,
}

pub struct GetMediaByURLInput<'a> {
    pub url: &'a Url,
    pub playlist_range: &'a Range,
    pub cache_search: &'a str,
    pub domain: Option<&'a str>,
    pub audio_language: &'a Language,
    pub sections: Option<&'a Sections>,
    pub tx_manager: &'a mut TxManager,
}

pub struct GetUncachedMediaByURLInput<'a> {
    pub url: &'a Url,
    pub playlist_range: &'a Range,
    pub audio_language: &'a Language,
}

pub enum GetMediaByURLKind {
    SingleCached(String),
    Playlist {
        cached: Vec<MediaInPlaylist>,
        uncached: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
    },
    Empty,
}

pub struct GetVideoByURL {
    pub router: Arc<NodeRouter>,
    pub tracking_params_cfg: Arc<TrackingParamsConfig>,
}

impl Interactor<GetMediaByURLInput<'_>> for &GetVideoByURL {
    type Output = GetMediaByURLKind;
    type Err = GetMediaByURLErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        GetMediaByURLInput {
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            tx_manager,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        get_media_by_url(
            self.router.as_ref(),
            self.tracking_params_cfg.as_ref(),
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            MediaType::Video,
            "video",
            tx_manager,
        )
        .await
    }
}

pub struct GetAudioByURL {
    pub router: Arc<NodeRouter>,
    pub tracking_params_cfg: Arc<TrackingParamsConfig>,
}

impl Interactor<GetMediaByURLInput<'_>> for &GetAudioByURL {
    type Output = GetMediaByURLKind;
    type Err = GetMediaByURLErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        GetMediaByURLInput {
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            tx_manager,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        get_media_by_url(
            self.router.as_ref(),
            self.tracking_params_cfg.as_ref(),
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            MediaType::Audio,
            "audio",
            tx_manager,
        )
        .await
    }
}

pub struct GetUncachedVideoByURL {
    pub router: Arc<NodeRouter>,
    pub tracking_params_cfg: Arc<TrackingParamsConfig>,
}

impl Interactor<GetUncachedMediaByURLInput<'_>> for &GetUncachedVideoByURL {
    type Output = Playlist;
    type Err = GetInfoErrorKind;

    async fn execute(
        self,
        GetUncachedMediaByURLInput {
            url,
            playlist_range,
            audio_language,
        }: GetUncachedMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Getting media");

        let response = fetch_media_info_with_retry(
            self.router.as_ref(),
            url.domain(),
            MediaInfoRequest {
                url: url.as_str().to_owned(),
                audio_language: audio_language.language.clone().unwrap_or_default(),
                playlist_range: Some(to_proto_range(playlist_range)),
                media_type: "video".to_owned(),
            },
        )
        .await?;
        let mut playlist = playlist_from_response(response)?;

        for (media, _) in &mut playlist.inner {
            media.remove_url_tracking_params(self.tracking_params_cfg.as_ref());
        }

        info!(playlist_len = playlist.inner.len(), "Got media");
        Ok(playlist)
    }
}

pub struct GetShortMediaByURL {
    pub client: Arc<Client>,
    pub yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

pub struct GetShortMediaByURLInput<'a> {
    pub url: &'a Url,
}

impl Interactor<GetShortMediaByURLInput<'_>> for &GetShortMediaByURL {
    type Output = Vec<BasicInfo>;
    type Err = GetVideoInfoErrorKind;

    #[instrument(skip_all)]
    async fn execute(self, GetShortMediaByURLInput { url }: GetShortMediaByURLInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Getting media");
        let res = get_video_info(&self.client, &self.yt_toolkit_cfg.url, url.as_str()).await?;
        info!("Got media");
        Ok(res)
    }
}

pub struct SearchMediaInfo {
    pub client: Arc<Client>,
    pub yt_toolkit_cfg: Arc<YtToolkitConfig>,
}

pub struct SearchMediaInfoInput<'a> {
    pub text: &'a str,
}

impl Interactor<SearchMediaInfoInput<'_>> for &SearchMediaInfo {
    type Output = Vec<BasicInfo>;
    type Err = SearchVideoErrorKind;

    #[instrument(skip_all)]
    async fn execute(self, SearchMediaInfoInput { text }: SearchMediaInfoInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Searching media");
        let res = search_video(&self.client, &self.yt_toolkit_cfg.url, text).await?;
        info!("Got media");
        Ok(res)
    }
}

#[allow(clippy::too_many_arguments)]
async fn get_media_by_url(
    router: &NodeRouter,
    tracking_params_cfg: &TrackingParamsConfig,
    url: &Url,
    playlist_range: &Range,
    cache_search: &str,
    domain: Option<&str>,
    audio_language: &Language,
    sections: Option<&Sections>,
    media_type: MediaType,
    media_type_str: &str,
    tx_manager: &mut TxManager,
) -> Result<GetMediaByURLKind, GetMediaByURLErrorKind> {
    tx_manager.begin().await.map_err(ErrorKind::from)?;

    let dao = tx_manager.downloaded_media_dao().unwrap();
    let is_single_media = playlist_range.is_single_element();
    let (start, end) = if let Some(sections) = sections {
        (sections.start, sections.end)
    } else {
        let val = Sections::default();
        (val.start, val.end)
    };

    if is_single_media {
        if let Some(media) = dao
            .get(
                cache_search,
                domain,
                audio_language.language.as_deref(),
                clone_media_type(&media_type),
                start,
                end,
            )
            .await?
        {
            info!("Got cached media");
            return Ok(GetMediaByURLKind::SingleCached(media.file_id));
        }
    }

    debug!("Getting media");

    let response = fetch_media_info_with_retry(
        router,
        domain,
        MediaInfoRequest {
            url: url.as_str().to_owned(),
            audio_language: audio_language.language.clone().unwrap_or_default(),
            playlist_range: Some(to_proto_range(playlist_range)),
            media_type: media_type_str.to_owned(),
        },
    )
    .await
    .map_err(map_get_media_info_error)?;
    let playlist = playlist_from_response(response)?;
    let playlist_len = playlist.inner.len();

    let mut cached = vec![];
    let mut uncached = vec![];
    for (mut media, mut formats) in playlist.inner {
        media.remove_url_tracking_params(tracking_params_cfg);
        let domain = media.webpage_url.domain();
        if let Some(DownloadedMedia { file_id, .. }) = dao
            .get(
                &media.id,
                domain,
                audio_language.language.as_deref(),
                clone_media_type(&media_type),
                start,
                end,
            )
            .await?
        {
            cached.push(MediaInPlaylist {
                file_id,
                playlist_index: media.playlist_index,
                webpage_url: Some(media.webpage_url.clone()),
            });
            continue;
        }
        formats.retain(|(format, _)| {
            if let Some(val) = format.filesize_approx {
                val <= router.max_file_size()
            } else {
                true
            }
        });
        uncached.push((media, formats));
    }

    if playlist_len == 0 {
        warn!("Empty playlist");
        return Ok(GetMediaByURLKind::Empty);
    }

    info!(playlist_len, cached_len = cached.len(), unchached_len = uncached.len(), "Got media");
    Ok(GetMediaByURLKind::Playlist { cached, uncached })
}

async fn fetch_media_info_with_retry(
    router: &NodeRouter,
    domain: Option<&str>,
    request: MediaInfoRequest,
) -> Result<ytdl_tg_bot_proto::downloader::MediaInfoResponse, GetInfoErrorKind> {
    let mut excluded = HashSet::new();

    loop {
        let Some(node) = router.pick_node_excluding(domain, &excluded) else {
            return Err(GetInfoErrorKind::NodeUnavailable);
        };

        node.reserve_download_slot();
        let result = async {
            let mut client = DownloaderClient::new(node.channel.clone()).max_decoding_message_size(MAX_DECODING_MESSAGE_SIZE);
            let response = client.get_media_info(authenticated_request(request.clone(), &node.token)?).await?;
            Ok::<_, GetInfoErrorKind>(response.into_inner())
        }
        .await;
        node.release_download_slot();

        match result {
            Ok(response) => return Ok(response),
            Err(GetInfoErrorKind::Rpc(status)) if status.code() == Code::ResourceExhausted => {
                excluded.insert(node.address.to_string());
            }
            Err(GetInfoErrorKind::Rpc(status)) if status.code() == Code::Aborted => {
                warn!(node = %node.address, %status, "Download node returned retryable yt-dlp HTTP 400");
                excluded.insert(node.address.to_string());
            }
            Err(GetInfoErrorKind::Rpc(status)) if status.code() == Code::Unavailable => {
                warn!(node = %node.address, %status, "Download node unavailable");
                excluded.insert(node.address.to_string());
            }
            Err(GetInfoErrorKind::Rpc(status)) if status.code() == Code::Unauthenticated => {
                error!(node = %node.address, %status, "Download node authentication failed");
                return Err(GetInfoErrorKind::NodeUnavailable);
            }
            Err(err) => return Err(err),
        }
    }
}

#[allow(clippy::result_large_err)]
fn playlist_from_response(response: ytdl_tg_bot_proto::downloader::MediaInfoResponse) -> Result<Playlist, GetInfoErrorKind> {
    let inner = response
        .entries
        .into_iter()
        .map(|entry| {
            let media = Media {
                id: entry.id,
                display_id: entry.display_id,
                webpage_url: Url::parse(&entry.webpage_url)?,
                title: entry.title,
                language: entry.audio_language,
                uploader: entry.uploader,
                duration: entry.duration,
                playlist_index: i16::try_from(entry.playlist_index)
                    .map_err(|_| GetInfoErrorKind::InvalidResponse("Invalid playlist_index".into()))?,
                thumbnail: None,
                thumbnails: vec![],
            };
            let formats = entry
                .formats
                .into_iter()
                .map(|format| {
                    (
                        MediaFormat {
                            format_id: format.format_id,
                            format_note: None,
                            ext: format.ext,
                            width: format.width,
                            height: format.height,
                            aspect_ratio: format.aspect_ratio,
                            filesize_approx: format.filesize_approx,
                        },
                        format.raw_info_json,
                    )
                })
                .collect();
            Ok::<_, GetInfoErrorKind>((media, formats))
        })
        .collect::<Result<_, _>>()?;

    Ok(Playlist { inner })
}

fn map_get_media_info_error(err: GetInfoErrorKind) -> GetMediaByURLErrorKind {
    match err {
        GetInfoErrorKind::NodeUnavailable => GetMediaByURLErrorKind::NodeUnavailable,
        err => GetMediaByURLErrorKind::GetInfo(err),
    }
}

fn to_proto_range(range: &Range) -> ytdl_tg_bot_proto::downloader::Range {
    ytdl_tg_bot_proto::downloader::Range {
        start: i32::from(range.start),
        count: i32::from(range.count),
        step: i32::from(range.step),
    }
}

fn clone_media_type(media_type: &MediaType) -> MediaType {
    match media_type {
        MediaType::Video => MediaType::Video,
        MediaType::Audio => MediaType::Audio,
    }
}
