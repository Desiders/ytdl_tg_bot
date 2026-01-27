use crate::{
    config::{TimeoutsConfig, YtDlpConfig, YtPotProviderConfig},
    entities::{Cookies, Media, MediaFormat, MediaInFS},
    interactors::Interactor,
    services::{
        download_and_convert,
        ytdl::{self, download_media},
    },
};
use std::{io, sync::Arc};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::{debug, error, info, info_span, instrument, warn};
use url::Url;

#[derive(thiserror::Error, Debug)]
pub enum DownloadMediaErrorKind {
    #[error(transparent)]
    Download(#[from] ytdl::DownloadErrorKind),
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadMediaPlaylistErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<(usize, Result<MediaInFS, DownloadMediaErrorKind>)>),
}

pub struct DownloadMediaInput<'a> {
    url: &'a Url,
    media: Media,
    format: MediaFormat,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
}

impl<'a> DownloadMediaInput<'a> {
    pub fn new_with_progress(url: &'a Url, media: Media, format: MediaFormat) -> (Self, mpsc::UnboundedReceiver<String>) {
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                media,
                format,
                progress_sender: Some(progress_sender),
            },
            progress_receiver,
        )
    }

    pub fn new(url: &'a Url, media: Media, format: MediaFormat) -> Self {
        Self {
            url,
            media,
            format,
            progress_sender: None,
        }
    }
}

pub struct DownloadMediaPlaylistInput<'a> {
    url: &'a Url,
    playlist: &'a [(Media, MediaFormat)],
    media_sender: mpsc::UnboundedSender<(usize, Result<MediaInFS, DownloadMediaErrorKind>)>,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
}

impl<'a> DownloadMediaPlaylistInput<'a> {
    #[allow(clippy::type_complexity)]
    pub fn new_with_progress(
        url: &'a Url,
        playlist: &'a [(Media, MediaFormat)],
    ) -> (
        Self,
        mpsc::UnboundedReceiver<(usize, Result<MediaInFS, DownloadMediaErrorKind>)>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                media_sender,
                progress_sender: Some(progress_sender),
            },
            media_receiver,
            progress_receiver,
        )
    }

    #[allow(clippy::type_complexity)]
    pub fn new(
        url: &'a Url,
        playlist: &'a [(Media, MediaFormat)],
    ) -> (Self, mpsc::UnboundedReceiver<(usize, Result<MediaInFS, DownloadMediaErrorKind>)>) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                media_sender,
                progress_sender: None,
            },
            media_receiver,
        )
    }
}

pub struct DownloadVideo {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadVideo {
    type Output = MediaInFS;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            format,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let format_id = &format.format_id;
        let max_file_size = self.yt_dlp_cfg.max_file_size;
        let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
        let output_dir_path = temp_dir.path();
        let output_file_path = output_dir_path.join(format!("{}.{}", media.id, format.ext));
        let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));
        let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
        let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
        let timeout = self.timeouts_cfg.video_download;
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        debug!("Downloading video");
        let thumb_urls = media.get_thumb_urls(format.aspect_ration_kind());
        let mut thumn_is_downloaded = false;
        for thumb_url in thumb_urls {
            match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                Ok(()) => {
                    info!("Thumb downloaded");
                    thumn_is_downloaded = true;
                    break;
                }
                Err(err) => {
                    warn!(%err, "Download thumb err");
                }
            }
        }

        download_media(
            media.webpage_url.as_str(),
            format_id,
            max_file_size,
            output_dir_path,
            executable_path,
            pot_provider_url,
            timeout,
            cookie,
            progress_sender,
        )
        .await?;
        info!("Video downloaded");

        Ok(Self::Output {
            path: output_file_path,
            thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
            temp_dir,
        })
    }
}

pub struct DownloadAudio {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadAudio {
    type Output = MediaInFS;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            format,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let format_id = &format.format_id;
        let max_file_size = self.yt_dlp_cfg.max_file_size;
        let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
        let output_dir_path = temp_dir.path();
        let output_file_path = output_dir_path.join(format!("{}.{}", media.id, format.ext));
        let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));
        let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
        let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
        let timeout = self.timeouts_cfg.video_download;
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        debug!("Downloading audio");
        let thumb_urls = media.get_thumb_urls(format.aspect_ration_kind());
        let mut thumn_is_downloaded = false;
        for thumb_url in thumb_urls {
            match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                Ok(()) => {
                    info!("Thumb downloaded");
                    thumn_is_downloaded = true;
                    break;
                }
                Err(err) => {
                    warn!(%err, "Download thumb err");
                }
            }
        }

        download_media(
            media.webpage_url.as_str(),
            format_id,
            max_file_size,
            output_dir_path,
            executable_path,
            pot_provider_url,
            timeout,
            cookie,
            progress_sender,
        )
        .await?;
        info!("Audio downloaded");

        Ok(Self::Output {
            path: output_file_path,
            thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
            temp_dir,
        })
    }
}

pub struct DownloadVideoPlaylist {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaPlaylistInput<'_>> for &DownloadVideoPlaylist {
    type Output = ();
    type Err = DownloadMediaPlaylistErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaPlaylistInput {
            url,
            playlist,
            media_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let max_file_size = self.yt_dlp_cfg.max_file_size;
        let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
        let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
        let timeout = self.timeouts_cfg.video_download;
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        for (index, (media, format)) in playlist.into_iter().enumerate() {
            let span = info_span!("iter", id = media.id, %format).entered();

            let format_id = &format.format_id;
            let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
            let output_dir_path = temp_dir.path();
            let output_file_path = output_dir_path.join(format!("{}.{}", media.id, format.ext));
            let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));

            debug!("Downloading video");
            let span = span.exit();

            let thumb_urls = media.get_thumb_urls(format.aspect_ration_kind());
            let mut thumn_is_downloaded = false;
            for thumb_url in thumb_urls {
                match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                    Ok(()) => {
                        let _guard = span.enter();
                        info!("Thumb downloaded");
                        thumn_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        let _guard = span.enter();
                        warn!(%err, "Download thumb err");
                    }
                }
            }

            if let Err(err) = download_media(
                media.webpage_url.as_str(),
                format_id,
                max_file_size,
                output_dir_path,
                executable_path,
                pot_provider_url,
                timeout,
                cookie,
                progress_sender.clone(),
            )
            .await
            {
                let _guard = span.enter();
                media_sender.send((index, Err(DownloadMediaErrorKind::Download(err))))?;
            }

            let _guard = span.enter();
            info!("Video downloaded");

            let media_in_fs = MediaInFS {
                path: output_file_path,
                thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
                temp_dir,
            };
            media_sender.send((index, Ok(media_in_fs)))?;
        }

        Ok(())
    }
}

pub struct DownloadAudioPlaylist {
    pub yt_dlp_cfg: Arc<YtDlpConfig>,
    pub yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
    pub cookies: Arc<Cookies>,
}

impl Interactor<DownloadMediaPlaylistInput<'_>> for &DownloadAudioPlaylist {
    type Output = ();
    type Err = DownloadMediaPlaylistErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaPlaylistInput {
            url,
            playlist,
            media_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let max_file_size = self.yt_dlp_cfg.max_file_size;
        let executable_path = self.yt_dlp_cfg.executable_path.as_ref();
        let pot_provider_url = self.yt_pot_provider_cfg.url.as_ref();
        let timeout = self.timeouts_cfg.video_download;
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        for (index, (media, format)) in playlist.into_iter().enumerate() {
            let span = info_span!("iter", id = media.id, %format).entered();

            let format_id = &format.format_id;
            let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
            let output_dir_path = temp_dir.path();
            let output_file_path = output_dir_path.join(format!("{}.{}", media.id, format.ext));
            let thumb_file_path = output_dir_path.join(format!("{}.jpg", media.id));

            debug!("Downloading audio");
            let span = span.exit();

            let thumb_urls = media.get_thumb_urls(format.aspect_ration_kind());
            let mut thumn_is_downloaded = false;
            for thumb_url in thumb_urls {
                match download_and_convert(thumb_url.as_str(), &thumb_file_path, "/usr/bin/ffmpeg", 5).await {
                    Ok(()) => {
                        let _guard = span.enter();
                        info!("Thumb downloaded");
                        thumn_is_downloaded = true;
                        break;
                    }
                    Err(err) => {
                        let _guard = span.enter();
                        warn!(%err, "Download thumb err");
                    }
                }
            }

            if let Err(err) = download_media(
                media.webpage_url.as_str(),
                format_id,
                max_file_size,
                output_dir_path,
                executable_path,
                pot_provider_url,
                timeout,
                cookie,
                progress_sender.clone(),
            )
            .await
            {
                let _guard = span.enter();
                media_sender.send((index, Err(DownloadMediaErrorKind::Download(err))))?;
            }

            let _guard = span.enter();
            info!("Audio downloaded");

            let media_in_fs = MediaInFS {
                path: output_file_path,
                thumb_path: if thumn_is_downloaded { Some(thumb_file_path) } else { None },
                temp_dir,
            };
            media_sender.send((index, Ok(media_in_fs)))?;
        }

        Ok(())
    }
}
