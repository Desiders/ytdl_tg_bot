use crate::{
    config::{YtDlpConfig, YtPotProviderConfig},
    entities::{AudioAndFormat, AudioInFS, Cookies},
    interactors::Interactor,
    services::{download_audio_to_path, download_thumbnail_to_path},
};

use std::{io, sync::Arc};
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::{event, instrument, span, Level};
use url::Url;

const DOWNLOAD_TIMEOUT: u64 = 180;

#[derive(thiserror::Error, Debug)]
pub enum DownloadAudioErrorKind {
    #[error("Ytdlp error: {0}")]
    Ytdlp(io::Error),
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadAudioPlaylistErrorKind {
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<(usize, Result<AudioInFS, DownloadAudioErrorKind>)>),
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
}

pub struct DownloadAudio {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl DownloadAudio {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct DownloadAudioInput<'a> {
    pub url: &'a Url,
    pub audio_and_format: AudioAndFormat<'a>,
}

impl<'a> DownloadAudioInput<'a> {
    pub const fn new(url: &'a Url, audio_and_format: AudioAndFormat<'a>) -> Self {
        Self { url, audio_and_format }
    }
}

impl Interactor for DownloadAudio {
    type Input<'a> = DownloadAudioInput<'a>;
    type Output = AudioInFS;
    type Err = DownloadAudioErrorKind;

    #[instrument(skip_all, fields(extension = format.codec.get_extension(), %format))]
    async fn execute(
        &mut self,
        DownloadAudioInput {
            audio_and_format: AudioAndFormat { video, format },
            url,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let extension = format.codec.get_extension();
        let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
        let file_path = temp_dir.path().join(format!("{video_id}.{extension}", video_id = video.id));
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());
        let thumbnail_urls = video.thumbnail_urls(host.as_ref());

        let (download_res, thumbnail_path) = tokio::join!(
            {
                let url = video.original_url.clone();
                let yt_dlp_executable_path = self.yt_dlp_cfg.executable_path.clone();
                let yt_pot_provider_url = self.yt_pot_provider_cfg.url.clone();
                let temp_dir_path = temp_dir.path().to_path_buf();
                async move {
                    let res = download_audio_to_path(
                        yt_dlp_executable_path,
                        url,
                        yt_pot_provider_url,
                        format.id,
                        extension,
                        temp_dir_path,
                        DOWNLOAD_TIMEOUT,
                        cookie,
                    )
                    .await;
                    event!(Level::INFO, "Audio downloaded");
                    res
                }
            },
            {
                let temp_dir_path = temp_dir.path().to_path_buf();
                async move {
                    for thumbnail_url in thumbnail_urls {
                        if let Some(thumbnail_path) = download_thumbnail_to_path(thumbnail_url, &video.id, &temp_dir_path).await {
                            event!(Level::INFO, "Thumbnail downloaded");
                            return Some(thumbnail_path);
                        }
                    }
                    None
                }
            }
        );
        if let Err(err) = download_res {
            return Err(Self::Err::Ytdlp(err));
        }

        Ok(AudioInFS::new(file_path, thumbnail_path, temp_dir))
    }
}

pub struct DownloadAudioPlaylist {
    yt_dlp_cfg: Arc<YtDlpConfig>,
    yt_pot_provider_cfg: Arc<YtPotProviderConfig>,
    cookies: Arc<Cookies>,
}

impl DownloadAudioPlaylist {
    pub const fn new(yt_dlp_cfg: Arc<YtDlpConfig>, yt_pot_provider_cfg: Arc<YtPotProviderConfig>, cookies: Arc<Cookies>) -> Self {
        Self {
            yt_dlp_cfg,
            yt_pot_provider_cfg,
            cookies,
        }
    }
}

pub struct DownloadAudioPlaylistInput<'a> {
    pub url: &'a Url,
    pub audios_and_formats: Vec<AudioAndFormat<'a>>,
    pub sender: mpsc::UnboundedSender<(usize, Result<AudioInFS, DownloadAudioErrorKind>)>,
}

impl<'a> DownloadAudioPlaylistInput<'a> {
    #[allow(clippy::type_complexity)]
    pub fn new(
        url: &'a Url,
        audios_and_formats: Vec<AudioAndFormat<'a>>,
    ) -> (Self, mpsc::UnboundedReceiver<(usize, Result<AudioInFS, DownloadAudioErrorKind>)>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                audios_and_formats,
                sender,
            },
            receiver,
        )
    }
}

impl Interactor for DownloadAudioPlaylist {
    type Input<'a> = DownloadAudioPlaylistInput<'a>;
    type Output = ();
    type Err = DownloadAudioPlaylistErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        &mut self,
        DownloadAudioPlaylistInput {
            url,
            audios_and_formats,
            sender,
        }: Self::Input<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let host = url.host();
        let cookie = self.cookies.get_path_by_optional_host(host.as_ref());

        for (index, AudioAndFormat { video, format }) in audios_and_formats.into_iter().enumerate() {
            let extension = format.codec.get_extension();

            let span = span!(Level::INFO, "iter", extension, %format).entered();
            let temp_dir = TempDir::new().map_err(Self::Err::TempDir)?;
            let file_path = temp_dir.path().join(format!("{video_id}.{extension}", video_id = video.id));
            let thumbnail_urls = video.thumbnail_urls(host.as_ref());

            let span = span.exit();
            let (download_res, thumbnail_path) = tokio::join!(
                {
                    let url = video.original_url.clone();
                    let yt_dlp_executable_path = self.yt_dlp_cfg.executable_path.clone();
                    let yt_pot_provider_url = self.yt_pot_provider_cfg.url.clone();
                    let temp_dir_path = temp_dir.path().to_path_buf();
                    async move {
                        let res = download_audio_to_path(
                            yt_dlp_executable_path,
                            url,
                            yt_pot_provider_url,
                            format.id,
                            extension,
                            temp_dir_path,
                            DOWNLOAD_TIMEOUT,
                            cookie,
                        )
                        .await;
                        event!(Level::INFO, "Audio downloaded");
                        res
                    }
                },
                {
                    let temp_dir_path = temp_dir.path().to_path_buf();
                    async move {
                        for thumbnail_url in thumbnail_urls {
                            if let Some(thumbnail_path) = download_thumbnail_to_path(thumbnail_url, &video.id, &temp_dir_path).await {
                                event!(Level::INFO, "Thumbnail downloaded");
                                return Some(thumbnail_path);
                            }
                        }
                        None
                    }
                }
            );
            if let Err(err) = download_res {
                let _guard = span.enter();
                sender.send((index, Err(DownloadAudioErrorKind::Ytdlp(err))))?;
                continue;
            }

            let _guard = span.enter();
            sender.send((index, Ok(AudioInFS::new(file_path, thumbnail_path, temp_dir))))?;
        }

        Ok(())
    }
}
