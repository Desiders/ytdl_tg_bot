use std::{convert::Infallible, sync::Arc};

use proto::downloader::{MediaInfoRequest, MediaInfoResponse};
use reqwest::Client;
use tracing::{debug, info, instrument, warn};
use url::Url;

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
        node_router::{
            get_media_info, resolve_to_drm_free, GetMediaInfoErrorKind as ClientGetMediaInfoErrorKind, NodeRouter,
            ResolveSourceErrorKind as ClientResolveSourceErrorKind,
        },
        yt_toolkit::{get_video_info, search_video, GetVideoInfoErrorKind, SearchVideoErrorKind},
    },
    value_objects::MediaType,
};

#[derive(Debug, thiserror::Error)]
pub enum GetInfoErrorKind {
    #[error(transparent)]
    Client(#[from] ClientGetMediaInfoErrorKind),
    #[error(transparent)]
    Url(#[from] url::ParseError),
    #[error("Invalid node response: {0}")]
    InvalidResponse(Box<str>),
}

#[derive(Debug, thiserror::Error)]
pub enum GetMediaByURLErrorKind {
    #[error(transparent)]
    GetInfo(#[from] GetInfoErrorKind),
    #[error(transparent)]
    Database(#[from] ErrorKind<Infallible>),
    #[error("All download nodes are busy. Try again later.")]
    NodeUnavailable,
    #[error(transparent)]
    Resolve(#[from] ClientResolveSourceErrorKind),
}

pub struct GetMediaByURLInput<'a> {
    pub url: &'a Url,
    pub playlist_range: &'a Range,
    pub cache_search: &'a str,
    pub domain: Option<&'a str>,
    pub audio_language: &'a Language,
    pub sections: Option<&'a Sections>,
    pub overwrite_cache: bool,
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
    node_router: Arc<NodeRouter>,
    cfg: Arc<TrackingParamsConfig>,
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetVideoByURL {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>, cfg: Arc<TrackingParamsConfig>, tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self {
            node_router,
            cfg,
            tx_manager,
        }
    }
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
            overwrite_cache,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        get_media_by_url(
            self.node_router.as_ref(),
            self.cfg.as_ref(),
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            overwrite_cache,
            MediaType::Video,
            "video",
            &**self.tx_manager,
        )
        .await
    }
}

pub struct GetAudioByURL {
    node_router: Arc<NodeRouter>,
    cfg: Arc<TrackingParamsConfig>,
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetAudioByURL {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>, cfg: Arc<TrackingParamsConfig>, tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self {
            node_router,
            cfg,
            tx_manager,
        }
    }
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
            overwrite_cache,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        get_media_by_url(
            self.node_router.as_ref(),
            self.cfg.as_ref(),
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            overwrite_cache,
            MediaType::Audio,
            "audio",
            &**self.tx_manager,
        )
        .await
    }
}

pub struct GetPhotoByURL {
    node_router: Arc<NodeRouter>,
    cfg: Arc<TrackingParamsConfig>,
    tx_manager: Arc<Box<dyn TxManager>>,
}

impl GetPhotoByURL {
    #[must_use]
    pub const fn new(node_router: Arc<NodeRouter>, cfg: Arc<TrackingParamsConfig>, tx_manager: Arc<Box<dyn TxManager>>) -> Self {
        Self {
            node_router,
            cfg,
            tx_manager,
        }
    }
}

impl Interactor<GetMediaByURLInput<'_>> for &GetPhotoByURL {
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
            overwrite_cache,
        }: GetMediaByURLInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        get_media_by_url(
            self.node_router.as_ref(),
            self.cfg.as_ref(),
            url,
            playlist_range,
            cache_search,
            domain,
            audio_language,
            sections,
            overwrite_cache,
            MediaType::Photo,
            "photo",
            &**self.tx_manager,
        )
        .await
    }
}

pub struct GetShortMediaByURL {
    client: Arc<Client>,
    cfg: Arc<YtToolkitConfig>,
}

impl GetShortMediaByURL {
    #[must_use]
    pub const fn new(client: Arc<Client>, cfg: Arc<YtToolkitConfig>) -> Self {
        Self { client, cfg }
    }
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
        let res = get_video_info(&self.client, &self.cfg.url, url.as_str()).await?;
        info!("Got media");
        Ok(res)
    }
}

pub struct SearchMediaInfo {
    client: Arc<Client>,
    cfg: Arc<YtToolkitConfig>,
}

impl SearchMediaInfo {
    #[must_use]
    pub const fn new(client: Arc<Client>, cfg: Arc<YtToolkitConfig>) -> Self {
        Self { client, cfg }
    }
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
        let res = search_video(&self.client, &self.cfg.url, text).await?;
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
    overwrite_cache: bool,
    media_type: MediaType,
    media_type_str: &str,
    tx_manager: &dyn TxManager,
) -> Result<GetMediaByURLKind, GetMediaByURLErrorKind> {
    // DRM music links (Spotify, Apple Music, ...) aren't downloadable; resolve them to DRM-free
    // sources first, then run the normal pipeline against those URLs. Non-DRM links are unchanged.
    // An album/playlist resolves to several URLs; the `items` range selects among them, like it
    // selects entries of a regular playlist on the node.
    let resolved_urls = if matches!(media_type, MediaType::Audio) {
        resolve_to_drm_free(router, url)
            .await?
            .map(|urls| select_by_range(urls, playlist_range))
    } else {
        None
    };
    if let Some([]) = resolved_urls.as_deref() {
        warn!("Empty playlist");
        return Ok(GetMediaByURLKind::Empty);
    }
    let single_resolved = resolved_urls.as_ref().and_then(|urls| match urls.as_slice() {
        [url] => Some(url),
        _ => None,
    });
    let url = single_resolved.unwrap_or(url);
    let domain = single_resolved.map_or(domain, |url: &Url| url.domain());
    let cache_search = single_resolved.map_or(cache_search, Url::as_str);

    let reader = tx_manager.downloaded_media_reader();
    let is_single_media = playlist_range.is_single_element();
    let (start, end) = if let Some(sections) = sections {
        (sections.start, sections.end)
    } else {
        let val = Sections::default();
        (val.start, val.end)
    };

    if is_single_media && !overwrite_cache {
        if let Some(media) = reader
            .get(cache_search, domain, audio_language.language.as_deref(), media_type, start, end)
            .await?
        {
            info!("Got cached media");
            return Ok(GetMediaByURLKind::SingleCached(media.file_id));
        }
    }

    debug!("Getting media");

    let map_get_info_err = |err| match GetInfoErrorKind::from(err) {
        GetInfoErrorKind::Client(ClientGetMediaInfoErrorKind::NodeUnavailable) => GetMediaByURLErrorKind::NodeUnavailable,
        err => GetMediaByURLErrorKind::GetInfo(err),
    };
    let response = match resolved_urls.as_deref() {
        // Several resolved tracks: each URL is a single video, so fetch them one by one (the range
        // was already applied to the URL list) and merge into one playlist-shaped response.
        Some(urls @ [_, _, ..]) => {
            let mut entries = vec![];
            for (index, url) in urls.iter().enumerate() {
                let response = get_media_info(
                    router,
                    url.domain(),
                    MediaInfoRequest {
                        url: url.as_str().to_owned(),
                        audio_language: audio_language.language.clone().unwrap_or_default(),
                        playlist_range: Some(Range::default().into()),
                        media_type: media_type_str.to_owned(),
                        max_file_size: router.max_file_size(),
                    },
                )
                .await
                .map_err(map_get_info_err)?;
                entries.extend(response.entries.into_iter().map(|mut entry| {
                    entry.playlist_index = i32::try_from(index).unwrap_or_default() + 1;
                    entry
                }));
            }
            MediaInfoResponse { entries }
        }
        _ => get_media_info(
            router,
            domain,
            MediaInfoRequest {
                url: url.as_str().to_owned(),
                audio_language: audio_language.language.clone().unwrap_or_default(),
                playlist_range: Some((*playlist_range).into()),
                media_type: media_type_str.to_owned(),
                max_file_size: router.max_file_size(),
            },
        )
        .await
        .map_err(map_get_info_err)?,
    };
    let playlist = playlist_from_response(response)?;
    let playlist_len = playlist.inner.len();

    let mut cached = vec![];
    let mut uncached = vec![];
    for (mut media, formats) in playlist.inner {
        media.remove_url_tracking_params(tracking_params_cfg);
        let domain = media.webpage_url.domain();
        if !overwrite_cache {
            if let Some(DownloadedMedia { file_id, .. }) = reader
                .get(&media.id, domain, audio_language.language.as_deref(), media_type, start, end)
                .await?
            {
                cached.push(MediaInPlaylist {
                    file_id,
                    playlist_index: media.playlist_index,
                    webpage_url: Some(media.webpage_url.clone()),
                });
                continue;
            }
        }
        uncached.push((media, formats));
    }

    if playlist_len == 0 {
        warn!("Empty playlist");
        return Ok(GetMediaByURLKind::Empty);
    }

    info!(playlist_len, cached_len = cached.len(), unchached_len = uncached.len(), "Got media");
    Ok(GetMediaByURLKind::Playlist { cached, uncached })
}

/// Applies the `items` range (1-based `start:count:step`) to the resolved URL list, mirroring how
/// the node applies it to a regular playlist's entries.
fn select_by_range(urls: Vec<Url>, range: &Range) -> Vec<Url> {
    urls.into_iter()
        .enumerate()
        .filter(|(index, _)| {
            let Ok(position) = i16::try_from(*index + 1) else {
                return false;
            };
            position >= range.start && position <= range.count && (position - range.start) % range.step == 0
        })
        .map(|(_, url)| url)
        .collect()
}

#[allow(clippy::result_large_err)]
fn playlist_from_response(response: proto::downloader::MediaInfoResponse) -> Result<Playlist, GetInfoErrorKind> {
    let inner = response
        .entries
        .into_iter()
        .map(|entry| {
            let direct_url = entry.direct_url.as_deref().map(Url::parse).transpose()?;
            let media = Media {
                id: entry.id,
                display_id: entry.display_id,
                webpage_url: Url::parse(&entry.webpage_url)?,
                direct_url,
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
