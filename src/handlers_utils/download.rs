use crate::{
    cmd::{download_audio_stream_to_pipe, download_audio_to_path, download_video_to_pipe, merge_streams, ytdl},
    fs::get_best_thumbnail_path_in_dir,
    models::{AudioInFS, VideoInFS, VideoInYT},
};

use nix::{
    fcntl::{fcntl, FcntlArg::F_SETFD, FdFlag},
    sys::wait::waitpid,
    unistd::{close, pipe},
};
use std::{io, path::Path};
use tracing::{event, field, instrument, Level, Span};

#[derive(thiserror::Error, Debug)]
pub enum StreamErrorKind {
    #[error("No format found for video {video_id}")]
    NoFormatFound { video_id: Box<str> },
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(not(target_family = "unix"))]
pub fn video(
    _video: VideoInYT,
    _video_id_or_url: impl AsRef<str>,
    _max_file_size: u64,
    _executable_ytdl_path: impl AsRef<str>,
    _temp_dir: &TempDir,
) -> Result<VideoInFS, StreamErrorKind> {
    unimplemented!("This function is only implemented for Unix systems");
}

#[cfg(target_family = "unix")]
#[instrument(skip_all, fields(video = %video_id_or_url.as_ref(), format_id, file_path))]
pub async fn video(
    video: VideoInYT,
    video_id_or_url: impl AsRef<str>,
    max_file_size: u64,
    executable_ytdl_path: impl AsRef<str>,
    temp_dir_path: impl AsRef<Path>,
) -> Result<VideoInFS, StreamErrorKind> {
    let mut combined_formats = video.get_combined_formats();
    combined_formats.sort_by_priority_and_skip_by_size(max_file_size);

    let Some(combined_format) = combined_formats.first().cloned() else {
        event!(Level::ERROR, %combined_formats, "No video format found");

        return Err(StreamErrorKind::NoFormatFound {
            video_id: video.id.into_boxed_str(),
        });
    };

    drop(combined_formats);

    Span::current().record("format_id", &combined_format.format_id());

    event!(Level::DEBUG, %combined_format, "Got combined format");

    // Create pipes to communicate between the yt-dl process and the ffmpeg process
    let (video_read_fd, video_write_fd) = pipe().map_err(io::Error::from)?;
    let (audio_read_fd, audio_write_fd) = pipe().map_err(io::Error::from)?;

    // Set the close-on-exec flag for the write ends of the pipes.
    // This is needed in multithreaded context,
    // because we use `fork` and we want to close the pipes in the parent process for all threads.
    // If we don't do this, we may get a deadlock
    fcntl(video_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
    fcntl(audio_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;

    let output_path = temp_dir_path.as_ref().join(format!("merged.{}", combined_format.get_extension()));

    let merge_child =
        merge_streams(video_read_fd, audio_read_fd, combined_format.get_extension(), &output_path).map_err(io::Error::from)?;

    // Set the close-on-exec flag for the read ends of the pipes.
    // We use this after `merge_streams` because we want to keep the pipes open in the ffmpeg process
    fcntl(video_read_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
    fcntl(audio_read_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;

    let _video_child = download_video_to_pipe(
        video_write_fd,
        &executable_ytdl_path,
        &video_id_or_url,
        combined_format.video_format.id,
    )?;
    let _audio_child = download_audio_stream_to_pipe(
        audio_write_fd,
        executable_ytdl_path,
        video_id_or_url,
        combined_format.audio_format.id,
    )?;

    // In multithreaded context, we need to close the pipes in the parent process for all threads
    // with the close-on-exec flag set, otherwise, we may get a deadlock.
    if let Err(errno) = close(video_write_fd) {
        event!(Level::WARN, %errno, "Error closing video write pipe");
    }
    if let Err(errno) = close(audio_write_fd) {
        event!(Level::WARN, %errno, "Error closing audio write pipe");
    }
    if let Err(errno) = close(video_read_fd) {
        event!(Level::WARN, %errno, "Error closing video read pipe");
    }
    if let Err(errno) = close(audio_read_fd) {
        event!(Level::WARN, %errno, "Error closing audio read pipe");
    }

    match waitpid(merge_child, None) {
        Ok(status) => {
            event!(Level::TRACE, ?status, "Merge process exited");
        }
        Err(errno) => {
            event!(Level::ERROR, %errno, "Error waiting for merge process");

            return Err(io::Error::from(errno).into());
        }
    }

    let thumbnail_path = get_best_thumbnail_path_in_dir(temp_dir_path, video.id).await?;

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

// #[instrument(skip_all, fields(video_id = video.id, format_id = field::Empty, file_path = field::Empty))]
// pub async fn video_to_temp_dir(
//     video: VideoInYT,
//     video_id_or_url: &str,
//     temp_dir: &TempDir,
//     max_file_size: u64,
//     executable_ytdl_path: &str,
//     allow_playlist: bool,
//     download_thumbnails: bool,
// ) -> Result<VideoInFS, DownloadToTempDirErrorKind> {
//     let mut combined_formats = video.get_combined_formats();
//     combined_formats.sort_by_priority_and_skip_by_size(max_file_size);

//     let Some(combined_format) = combined_formats.first().cloned() else {
//         event!(Level::ERROR, %combined_formats, "No format found for video");

//         return Err(DownloadToTempDirErrorKind::NoFormatFound {
//             video_id: video.id.into_boxed_str(),
//         });
//     };

//     drop(combined_formats);

//     let extension = combined_format.get_extension();

//     Span::current().record("format_id", &combined_format.format_id());

//     event!(Level::DEBUG, %combined_format, "Got combined format");

//     let file_path = temp_dir.path().join(format!("{video_id}.{extension}", video_id = video.id));

//     Span::current().record("file_path", file_path.display().to_string());

//     event!(Level::DEBUG, ?file_path, "Got file path");

//     download_video_to_path(
//         executable_ytdl_path,
//         temp_dir.path().to_string_lossy().as_ref(),
//         video_id_or_url,
//         combined_format.format_id().as_ref(),
//         extension,
//         allow_playlist,
//         download_thumbnails,
//     )
//     .await?;

//     event!(Level::DEBUG, "Video downloaded");

//     let thumbnail_path = if download_thumbnails {
//         get_best_thumbnail_path_in_dir(temp_dir.path(), video.id.as_str()).await?
//     } else {
//         None
//     };

//     event!(Level::TRACE, ?thumbnail_path, "Got thumbnail path");

//     Ok(VideoInFS::new(file_path, thumbnail_path))
// }

#[instrument(skip_all, fields(video = video.id, format_id = field::Empty, file_path = field::Empty))]
pub async fn audio_to_temp_dir(
    video: VideoInYT,
    video_id_or_url: impl AsRef<str>,
    max_file_size: u64,
    executable_ytdl_path: impl AsRef<str>,
    temp_dir_path: impl AsRef<Path>,
) -> Result<AudioInFS, ToTempDirErrorKind> {
    let mut audio_formats = video.get_audio_formats();
    audio_formats.sort_by_priority_and_skip_by_size(max_file_size);

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

    download_audio_to_path(executable_ytdl_path, video_id_or_url, audio_format.id, extension, &temp_dir_path).await?;

    event!(Level::DEBUG, "Audio downloaded");

    let thumbnail_path = get_best_thumbnail_path_in_dir(temp_dir_path, video.id).await?;

    Ok(AudioInFS::new(file_path, thumbnail_path))
}
