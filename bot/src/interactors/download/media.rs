use std::{collections::HashSet, io, path::Path, sync::Arc};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt as _, sync::mpsc};
use tonic::Code;
use tracing::{error, instrument, warn};
use url::Url;
use ytdl_tg_bot_proto::downloader::{download_chunk::Payload, downloader_client::DownloaderClient, DownloadRequest, Section};

use crate::{
    entities::{Media, MediaFormat, MediaInFS, RawMediaWithFormat, Sections},
    interactors::Interactor,
    node_router::{authenticated_request, NodeHandle, NodeRouter},
};

#[derive(thiserror::Error, Debug)]
pub enum DownloadErrorKind {
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error("Invalid download stream")]
    InvalidStream,
    #[error("No download node available")]
    NodeUnavailable,
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadMediaErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Media file error: {0}")]
    MediaFile(io::Error),
    #[error("Channel error: {0}")]
    Channel(#[from] mpsc::error::SendError<DownloadErrorKind>),
    #[error(transparent)]
    Download(#[from] DownloadErrorKind),
}

#[derive(thiserror::Error, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum DownloadMediaPlaylistErrorKind {
    #[error("Temp dir error: {0}")]
    TempDir(io::Error),
    #[error("Media file error: {0}")]
    MediaFile(io::Error),
    #[error("Channel error: {0}")]
    ErrChannel(#[from] mpsc::error::SendError<Vec<DownloadErrorKind>>),
    #[error("Channel error: {0}")]
    MediaChannel(#[from] mpsc::error::SendError<(MediaInFS, Media, MediaFormat)>),
}

pub struct DownloadMediaInput<'a> {
    url: &'a Url,
    media: &'a Media,
    sections: Option<&'a Sections>,
    formats: Vec<(MediaFormat, RawMediaWithFormat)>,
    err_sender: mpsc::UnboundedSender<DownloadErrorKind>,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
}

impl<'a> DownloadMediaInput<'a> {
    pub fn new_with_progress(
        url: &'a Url,
        media: &'a Media,
        sections: Option<&'a Sections>,
        formats: Vec<(MediaFormat, RawMediaWithFormat)>,
    ) -> (Self, mpsc::UnboundedReceiver<DownloadErrorKind>, mpsc::UnboundedReceiver<String>) {
        let (err_sender, err_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                media,
                sections,
                formats,
                err_sender,
                progress_sender: Some(progress_sender),
            },
            err_receiver,
            progress_receiver,
        )
    }
}

pub struct DownloadMediaPlaylistInput<'a> {
    url: &'a Url,
    playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
    sections: Option<&'a Sections>,
    media_sender: mpsc::UnboundedSender<(MediaInFS, Media, MediaFormat)>,
    errs_sender: Option<mpsc::UnboundedSender<Vec<DownloadErrorKind>>>,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
}

impl<'a> DownloadMediaPlaylistInput<'a> {
    #[allow(clippy::type_complexity)]
    pub fn new_with_progress(
        url: &'a Url,
        playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
        sections: Option<&'a Sections>,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<(MediaInFS, Media, MediaFormat)>,
        mpsc::UnboundedReceiver<Vec<DownloadErrorKind>>,
        mpsc::UnboundedReceiver<String>,
    ) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        let (errs_sender, errs_receiver) = mpsc::unbounded_channel();
        let (progress_sender, progress_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                sections,
                media_sender,
                errs_sender: Some(errs_sender),
                progress_sender: Some(progress_sender),
            },
            media_receiver,
            errs_receiver,
            progress_receiver,
        )
    }

    #[allow(clippy::type_complexity)]
    pub fn new(
        url: &'a Url,
        playlist: Vec<(Media, Vec<(MediaFormat, RawMediaWithFormat)>)>,
        sections: Option<&'a Sections>,
    ) -> (Self, mpsc::UnboundedReceiver<(MediaInFS, Media, MediaFormat)>) {
        let (media_sender, media_receiver) = mpsc::unbounded_channel();
        (
            Self {
                url,
                playlist,
                sections,
                media_sender,
                errs_sender: None,
                progress_sender: None,
            },
            media_receiver,
        )
    }
}

pub struct DownloadVideo {
    pub router: Arc<NodeRouter>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadVideo {
    type Output = Option<(MediaInFS, MediaFormat)>;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            sections,
            formats,
            err_sender,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;

        for (format, raw) in formats {
            let request = build_download_request(url, &format, raw, "video", "", sections, self.router.max_file_size());

            match download_with_retry(
                self.router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
            )
            .await
            {
                Ok((path, format)) => {
                    let media_in_fs = MediaInFS {
                        path,
                        thumb_path: None,
                        temp_dir,
                    };
                    return Ok(Some((media_in_fs, format)));
                }
                Err(AttemptError::Download(err)) => {
                    err_sender.send(err)?;
                }
                Err(AttemptError::MediaFile(err)) => return Err(Self::Err::MediaFile(err)),
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadAudio {
    pub router: Arc<NodeRouter>,
}

impl Interactor<DownloadMediaInput<'_>> for &DownloadAudio {
    type Output = Option<(MediaInFS, MediaFormat)>;
    type Err = DownloadMediaErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        DownloadMediaInput {
            url,
            media,
            sections,
            formats,
            err_sender,
            progress_sender,
        }: DownloadMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;

        for (format, raw) in formats {
            let request = build_download_request(url, &format, raw, "audio", "m4a", sections, self.router.max_file_size());

            match download_with_retry(
                self.router.as_ref(),
                url.domain(),
                request,
                temp_dir.path(),
                &format,
                progress_sender.as_ref(),
            )
            .await
            {
                Ok((path, format)) => {
                    let media_in_fs = MediaInFS {
                        path,
                        thumb_path: None,
                        temp_dir,
                    };
                    return Ok(Some((media_in_fs, format)));
                }
                Err(AttemptError::Download(err)) => {
                    err_sender.send(err)?;
                }
                Err(AttemptError::MediaFile(err)) => return Err(Self::Err::MediaFile(err)),
            }
        }

        let _ = media;
        Ok(None)
    }
}

pub struct DownloadVideoPlaylist {
    pub router: Arc<NodeRouter>,
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
            sections,
            media_sender,
            errs_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        for (media, formats) in playlist {
            let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;
            let mut errs = vec![];
            let mut media_is_downloaded = false;

            for (format, raw) in formats {
                let request = build_download_request(url, &format, raw, "video", "", sections, self.router.max_file_size());

                match download_with_retry(
                    self.router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                )
                .await
                {
                    Ok((path, format)) => {
                        let media_in_fs = MediaInFS {
                            path,
                            thumb_path: None,
                            temp_dir,
                        };
                        media_sender.send((media_in_fs, media, format))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(AttemptError::Download(err)) => {
                        errs.push(err);
                    }
                    Err(AttemptError::MediaFile(err)) => return Err(Self::Err::MediaFile(err)),
                }
            }

            if let Some(ref sender) = errs_sender {
                if !media_is_downloaded {
                    sender.send(errs)?;
                }
            }
        }

        Ok(())
    }
}

pub struct DownloadAudioPlaylist {
    pub router: Arc<NodeRouter>,
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
            sections,
            media_sender,
            errs_sender,
            progress_sender,
        }: DownloadMediaPlaylistInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        for (media, formats) in playlist {
            let temp_dir = TempDir::with_prefix("ytdl-tg-bot-").map_err(Self::Err::TempDir)?;
            let mut errs = vec![];
            let mut media_is_downloaded = false;

            for (format, raw) in formats {
                let request = build_download_request(url, &format, raw, "audio", "m4a", sections, self.router.max_file_size());

                match download_with_retry(
                    self.router.as_ref(),
                    url.domain(),
                    request,
                    temp_dir.path(),
                    &format,
                    progress_sender.as_ref(),
                )
                .await
                {
                    Ok((path, format)) => {
                        let media_in_fs = MediaInFS {
                            path,
                            thumb_path: None,
                            temp_dir,
                        };
                        media_sender.send((media_in_fs, media, format))?;
                        media_is_downloaded = true;
                        break;
                    }
                    Err(AttemptError::Download(err)) => {
                        errs.push(err);
                    }
                    Err(AttemptError::MediaFile(err)) => return Err(Self::Err::MediaFile(err)),
                }
            }

            if let Some(ref sender) = errs_sender {
                if !media_is_downloaded {
                    sender.send(errs)?;
                }
            }
        }

        Ok(())
    }
}

enum AttemptError {
    Download(DownloadErrorKind),
    MediaFile(io::Error),
}

async fn download_with_retry(
    router: &NodeRouter,
    domain: Option<&str>,
    request: DownloadRequest,
    output_dir: &Path,
    base_format: &MediaFormat,
    progress_sender: Option<&mpsc::UnboundedSender<String>>,
) -> Result<(std::path::PathBuf, MediaFormat), AttemptError> {
    let mut excluded = HashSet::new();

    loop {
        let Some(node) = router.pick_node_excluding(domain, &excluded) else {
            return Err(AttemptError::Download(DownloadErrorKind::NodeUnavailable));
        };

        node.reserve_download_slot();
        let result = download_from_node(node.clone(), request.clone(), output_dir, base_format, progress_sender).await;
        node.release_download_slot();

        match result {
            Ok(result) => return Ok(result),
            Err(AttemptError::Download(DownloadErrorKind::Rpc(status))) if status.code() == Code::ResourceExhausted => {
                excluded.insert(node.address.to_string());
            }
            Err(AttemptError::Download(DownloadErrorKind::Rpc(status))) if status.code() == Code::Unavailable => {
                warn!(node = %node.address, %status, "Download node unavailable");
                excluded.insert(node.address.to_string());
            }
            Err(AttemptError::Download(DownloadErrorKind::Rpc(status))) if status.code() == Code::Unauthenticated => {
                error!(node = %node.address, %status, "Download node authentication failed");
                return Err(AttemptError::Download(DownloadErrorKind::NodeUnavailable));
            }
            Err(other) => return Err(other),
        }
    }
}

async fn download_from_node(
    node: Arc<NodeHandle>,
    request: DownloadRequest,
    output_dir: &Path,
    base_format: &MediaFormat,
    progress_sender: Option<&mpsc::UnboundedSender<String>>,
) -> Result<(std::path::PathBuf, MediaFormat), AttemptError> {
    let mut client = DownloaderClient::new(node.channel.clone());
    let response = client
        .download_media(
            authenticated_request(request, &node.token)
                .map_err(DownloadErrorKind::from)
                .map_err(AttemptError::Download)?,
        )
        .await
        .map_err(DownloadErrorKind::from)
        .map_err(AttemptError::Download)?;
    let mut stream = response.into_inner();

    let first = stream
        .message()
        .await
        .map_err(DownloadErrorKind::from)
        .map_err(AttemptError::Download)?
        .ok_or_else(|| AttemptError::Download(DownloadErrorKind::InvalidStream))?;
    let Payload::Meta(meta) = first
        .payload
        .ok_or_else(|| AttemptError::Download(DownloadErrorKind::InvalidStream))?
    else {
        return Err(AttemptError::Download(DownloadErrorKind::InvalidStream));
    };

    let path = output_dir.join(format!("media.{}", meta.ext));
    let mut file = tokio::fs::File::create(&path).await.map_err(AttemptError::MediaFile)?;
    let mut format = base_format.clone();
    format.ext = meta.ext;
    format.width = meta.width;
    format.height = meta.height;

    while let Some(chunk) = stream
        .message()
        .await
        .map_err(DownloadErrorKind::from)
        .map_err(AttemptError::Download)?
    {
        match chunk.payload {
            Some(Payload::Progress(progress)) => {
                if let Some(sender) = progress_sender {
                    let _ = sender.send(progress);
                }
            }
            Some(Payload::Data(data)) => {
                file.write_all(&data).await.map_err(AttemptError::MediaFile)?;
            }
            Some(Payload::Meta(_)) | None => {
                return Err(AttemptError::Download(DownloadErrorKind::InvalidStream));
            }
        }
    }

    file.flush().await.map_err(AttemptError::MediaFile)?;
    drop(file);

    Ok((path, format))
}

fn build_download_request(
    url: &Url,
    format: &MediaFormat,
    raw_info_json: String,
    media_type: &str,
    audio_ext: &str,
    sections: Option<&Sections>,
    max_file_size: u64,
) -> DownloadRequest {
    DownloadRequest {
        url: url.as_str().to_owned(),
        format_id: format.format_id.clone(),
        raw_info_json,
        media_type: media_type.to_owned(),
        audio_ext: audio_ext.to_owned(),
        section: sections.map(|sections| Section {
            start: sections.start,
            end: sections.end,
        }),
        max_file_size,
    }
}
