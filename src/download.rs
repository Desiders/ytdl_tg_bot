use crate::{
    entities::{AudioInFS, Cookie, ShortInfo, Video, VideoInFS},
    services::{
        convert_to_jpg, download_audio_to_path, download_to_pipe, download_video_to_path, get_best_thumbnail_path_in_dir, merge_streams,
        yt_toolkit, ytdl,
    },
    utils::format_error_report,
};
use futures_util::StreamExt as _;
use nix::{
    fcntl::{fcntl, FcntlArg::F_SETFD, FdFlag},
    unistd::pipe,
};
use reqwest::Client;
use std::{
    borrow::Cow,
    fs::File,
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{io::AsyncWriteExt, task::JoinError, time::timeout};
use tracing::{event, field, instrument, Level, Span};

#[derive(thiserror::Error, Debug)]
pub enum RangeDownloadKind {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum StreamErrorKind {
    #[error("No format found for video {video_id}")]
    NoFormatFound { video_id: Box<str> },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    RangeDownload(#[from] RangeDownloadKind),
    #[error(transparent)]
    Join(#[from] JoinError),
}

#[instrument(skip_all)]
pub fn get_thumbnail_url<'a>(video: &'a ShortInfo, yt_toolkit_api_url: impl AsRef<str>, is_youtube: bool) -> Option<Cow<'a, str>> {
    if is_youtube {
        match (video.width, video.height) {
            (Some(width), Some(height)) => match yt_toolkit::get_thumbnail_url(yt_toolkit_api_url.as_ref(), &video.id, width, height) {
                Ok(url) => Some(Cow::Owned(url)),
                Err(_) => video.thumbnail().map(Cow::Borrowed),
            },
            _ => video.thumbnail().map(Cow::Borrowed),
        }
    } else {
        video.thumbnail().map(Cow::Borrowed)
    }
}

#[instrument(skip_all)]
async fn get_thumbnail_path(url: impl AsRef<str>, id: impl AsRef<str>, temp_dir_path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = temp_dir_path.as_ref().join(format!("{}.jpg", id.as_ref()));

    match convert_to_jpg(url, &path).await {
        Ok(mut child) => match timeout(Duration::from_secs(10), child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    Some(path)
                } else {
                    None
                }
            }
            Ok(Err(err)) => {
                event!(Level::ERROR, err = format_error_report(&err), "Failed to convert thumbnail");
                None
            }
            Err(_) => {
                event!(Level::WARN, "Convert thumbnail timed out");
                None
            }
        },
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Failed to convert thumbnail");
            None
        }
    }
}

const RANGE_CHUNK_SIZE: i32 = 1024 * 1024 * 10;

async fn range_download_to_write<W: AsyncWriteExt + Unpin>(
    url: impl AsRef<str>,
    filesize: f64,
    mut write: W,
) -> Result<(), RangeDownloadKind> {
    let client = Client::new();
    let url = url.as_ref();

    let mut start = 0;
    let mut end = RANGE_CHUNK_SIZE;

    loop {
        event!(Level::TRACE, start, end, "Download chunk");

        #[allow(clippy::cast_possible_truncation)]
        if end >= filesize as i32 {
            let mut stream = client
                .get(url)
                .header("Range", format!("bytes={start}-"))
                .send()
                .await?
                .bytes_stream();

            while let Some(chunk_res) = stream.next().await {
                let chunk = chunk_res?;
                write.write_all(&chunk).await?;
            }

            break;
        }

        let mut stream = client
            .get(url)
            .header("Range", format!("bytes={start}-{end}"))
            .send()
            .await?
            .bytes_stream();

        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            write.write_all(&chunk).await?;
        }

        start = end + 1;
        end += RANGE_CHUNK_SIZE;
    }

    Ok(())
}

#[instrument(skip_all, fields(url = %video.original_url, format_id, file_path, extension))]
#[allow(clippy::unnecessary_to_owned)]
pub async fn video(
    video: Video,
    max_file_size: u32,
    executable_ytdl_path: impl AsRef<str>,
    yt_toolkit_api_url: impl AsRef<str>,
    yt_pot_provider_api_url: impl AsRef<str>,
    temp_dir_path: impl AsRef<Path>,
    is_youtube: bool,
    download_and_merge_timeout: u64,
    cookie: Option<&Cookie>,
    preferred_languages: &[&str],
) -> Result<VideoInFS, StreamErrorKind> {
    let mut combined_formats = video.get_combined_formats();
    combined_formats.sort(max_file_size, preferred_languages);

    let Some(combined_format) = combined_formats.first().cloned() else {
        event!(Level::WARN, %combined_formats, "No video format found");

        return Err(StreamErrorKind::NoFormatFound {
            video_id: video.id.into_boxed_str(),
        });
    };

    drop(combined_formats);

    let extension = combined_format.get_extension();

    Span::current().record("format_id", combined_format.format_id());
    Span::current().record("extension", extension);

    event!(Level::DEBUG, %combined_format, "Got combined format");

    // If formats are the same, we need to download it directly without merge audio and video using FFmpeg
    if combined_format.format_ids_are_equal() {
        event!(Level::DEBUG, "Video and audio formats are the same");

        let file_path = temp_dir_path.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));

        Span::current().record("file_path", file_path.display().to_string());

        let (thumbnail_path, download_thumbnails) =
            if let Some(thumbnail_url) = get_thumbnail_url(&video.clone().into(), yt_toolkit_api_url, is_youtube) {
                let thumbnail_path = get_thumbnail_path(thumbnail_url, &video.id, &temp_dir_path).await;
                (thumbnail_path, false)
            } else {
                (None, true)
            };

        download_video_to_path(
            executable_ytdl_path,
            &video.original_url,
            yt_pot_provider_api_url.as_ref(),
            extension,
            &temp_dir_path,
            download_and_merge_timeout,
            download_thumbnails,
            cookie,
        )
        .await?;

        let thumbnail_path = match thumbnail_path {
            Some(url) => Some(url),
            None => {
                if download_thumbnails {
                    get_best_thumbnail_path_in_dir(&temp_dir_path).ok().flatten()
                } else {
                    None
                }
            }
        };

        return Ok(VideoInFS::new(file_path, thumbnail_path));
    }

    event!(Level::DEBUG, "Video and audio formats are different");

    let (video_read_fd, video_write_fd) = pipe().map_err(io::Error::from)?;
    let (audio_read_fd, audio_write_fd) = pipe().map_err(io::Error::from)?;

    fcntl(&video_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
    fcntl(&audio_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;

    let output_path = temp_dir_path.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));

    let merge_child = merge_streams(&video_read_fd, &audio_read_fd, extension.to_owned(), output_path.clone());

    if let Some(filesize) = combined_format.video_format.filesize_or_approx() {
        tokio::spawn({
            let url = combined_format.video_format.url.to_owned();
            async move {
                range_download_to_write(url, filesize, tokio::fs::File::from_std(File::from(video_write_fd)))
                    .await
                    .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)))
            }
        });
    } else {
        download_to_pipe(
            video_write_fd,
            &executable_ytdl_path,
            &video.original_url,
            yt_pot_provider_api_url.as_ref(),
            combined_format.video_format.id,
            cookie,
        )?;
    }

    if let Some(filesize) = combined_format.audio_format.filesize_or_approx() {
        tokio::spawn({
            let url = combined_format.audio_format.url.to_owned();
            async move {
                range_download_to_write(url, filesize, tokio::fs::File::from_std(File::from(audio_write_fd)))
                    .await
                    .map_err(|err| event!(Level::ERROR, "{}", format_error_report(&err)))
            }
        });
    } else {
        download_to_pipe(
            audio_write_fd,
            &executable_ytdl_path,
            &video.original_url,
            yt_pot_provider_api_url.as_ref(),
            combined_format.audio_format.id,
            cookie,
        )?;
    }

    let thumbnail_path = if let Some(thumbnail_url) = get_thumbnail_url(&video.clone().into(), yt_toolkit_api_url, is_youtube) {
        get_thumbnail_path(thumbnail_url, &video.id, &temp_dir_path).await
    } else {
        None
    };

    let exit_code = match timeout(Duration::from_secs(download_and_merge_timeout), merge_child?.wait()).await {
        Ok(Ok(exit_code)) => exit_code,
        Ok(Err(err)) => {
            event!(Level::ERROR, "FFmpeg process IO error");

            return Err(err.into());
        }
        Err(_) => {
            event!(Level::ERROR, "FFmpeg process timed out");

            return Err(io::Error::new(io::ErrorKind::TimedOut, "FFmpeg process timed out").into());
        }
    };

    if !exit_code.success() {
        event!(Level::ERROR, "FFmpeg exited with status `{exit_code}`");

        return Err(io::Error::other(format!("FFmpeg exited with status `{exit_code}`")).into());
    }

    event!(Level::DEBUG, "Streams merged");

    Ok(VideoInFS::new(output_path, thumbnail_path))
}

#[derive(thiserror::Error, Debug)]
pub enum ToTempDirErrorKind {
    #[error("No format found for video {video_id}")]
    NoFormatFound { video_id: Box<str> },
    #[error(transparent)]
    Ytdl(#[from] ytdl::Error),
    #[error("Failed to get best thumbnail path in dir: {0}")]
    ThumbnailPathFailed(#[from] io::Error),
}

#[instrument(skip_all, fields(video = video.id, format_id = field::Empty, file_path = field::Empty))]
pub async fn audio_to_temp_dir(
    video: Video,
    video_id_or_url: impl AsRef<str>,
    max_file_size: u32,
    executable_ytdl_path: impl AsRef<str>,
    yt_toolkit_api_url: impl AsRef<str>,
    yt_pot_provider_api_url: impl AsRef<str>,
    temp_dir_path: impl AsRef<Path>,
    is_youtube: bool,
    download_timeout: u64,
    cookie: Option<&Cookie>,
    preferred_languages: &[&str],
) -> Result<AudioInFS, ToTempDirErrorKind> {
    let mut audio_formats = video.get_audio_formats();
    audio_formats.sort(max_file_size, preferred_languages);

    let Some(audio_format) = audio_formats.first().cloned() else {
        event!(Level::ERROR, ?audio_formats, "No format found for audio");

        return Err(ToTempDirErrorKind::NoFormatFound {
            video_id: video.id.into_boxed_str(),
        });
    };

    drop(audio_formats);

    let extension = audio_format.codec.get_extension();

    Span::current().record("format_id", audio_format.id);

    event!(Level::DEBUG, %audio_format, "Got audio format");

    let file_path = temp_dir_path.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));

    Span::current().record("file_path", file_path.display().to_string());

    event!(Level::DEBUG, ?file_path, "Got file path");

    let (thumbnail_path, download_thumbnails) =
        if let Some(thumbnail_url) = get_thumbnail_url(&video.clone().into(), yt_toolkit_api_url, is_youtube) {
            let thumbnail_path = get_thumbnail_path(thumbnail_url, &video.id, &temp_dir_path).await;
            (thumbnail_path, false)
        } else {
            (None, true)
        };

    download_audio_to_path(
        executable_ytdl_path,
        video_id_or_url,
        yt_pot_provider_api_url.as_ref(),
        audio_format.id,
        extension,
        &temp_dir_path,
        download_timeout,
        download_thumbnails,
        cookie,
    )
    .await?;

    event!(Level::DEBUG, "Audio downloaded");

    let thumbnail_path = match thumbnail_path {
        Some(url) => Some(url),
        None => {
            if download_thumbnails {
                get_best_thumbnail_path_in_dir(&temp_dir_path).ok().flatten()
            } else {
                None
            }
        }
    };

    Ok(AudioInFS::new(file_path, thumbnail_path))
}
