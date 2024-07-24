use crate::{
    cmd::{convert_to_jpg, download_audio_to_path, download_to_pipe, download_video_to_path, merge_streams, ytdl},
    fs::get_best_thumbnail_path_in_dir,
    models::{AudioInFS, VideoInFS, VideoInYT},
};
use nix::{
    fcntl::{fcntl, FcntlArg::F_SETFD, FdFlag},
    unistd::{close, pipe},
};
use std::{
    io,
    os::fd::{FromRawFd as _, OwnedFd},
    path::{Path, PathBuf},
    time::Duration,
};
use tracing::{event, field, instrument, Level, Span};
use wait_timeout::ChildExt as _;

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

fn get_thumbnail_path(url: impl AsRef<str>, id: impl AsRef<str>, temp_dir_path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = temp_dir_path.as_ref().join(format!("{}.jpg", id.as_ref()));

    match convert_to_jpg(url, &path) {
        Ok(()) => Some(path),
        Err(err) => {
            event!(Level::ERROR, %err, "Error downloading thumbnail");

            // We don't want to fail the whole process if the thumbnail download fails
            None
        }
    }
}

#[cfg(target_family = "unix")]
#[instrument(skip_all, fields(video = %video_id_or_url.as_ref(), format_id, file_path, extension))]
pub fn video(
    video: VideoInYT,
    video_id_or_url: impl AsRef<str>,
    max_file_size: u64,
    executable_ytdl_path: impl AsRef<str>,
    temp_dir_path: impl AsRef<Path>,
    timeout: u64,
) -> Result<VideoInFS, StreamErrorKind> {
    let mut combined_formats = video.get_combined_formats();
    combined_formats.sort_by_priority_and_skip_by_size(max_file_size);

    let Some(combined_format) = combined_formats.first().cloned() else {
        event!(Level::WARN, %combined_formats, "No video format found");

        return Err(StreamErrorKind::NoFormatFound {
            video_id: video.id.into_boxed_str(),
        });
    };

    drop(combined_formats);

    let extension = combined_format.get_extension();

    Span::current().record("format_id", &combined_format.format_id());
    Span::current().record("extension", extension);

    event!(Level::DEBUG, %combined_format, "Got combined format");

    // If formats are the same, we need to download it directly without merge audio and video using FFmpeg
    if combined_format.format_ids_are_equal() {
        event!(Level::DEBUG, "Video and audio formats are the same");

        let file_path = temp_dir_path.as_ref().join(format!("{video_id}.{extension}", video_id = video.id));

        Span::current().record("file_path", file_path.display().to_string());

        download_video_to_path(&executable_ytdl_path, &video_id_or_url, extension, &temp_dir_path, timeout)?;

        let thumbnail_path = video
            .thumbnail
            .map(|url| get_thumbnail_path(url, video.id, &temp_dir_path))
            .flatten()
            .or_else(|| get_best_thumbnail_path_in_dir(&temp_dir_path).ok().flatten());

        return Ok(VideoInFS::new(file_path, thumbnail_path));
    }

    event!(Level::DEBUG, "Video and audio formats are different");

    // Create pipes to communicate between the yt-dl process and the ffmpeg process
    let (video_read_fd, video_write_fd) = pipe().map_err(io::Error::from)?;
    let (audio_read_fd, audio_write_fd) = pipe().map_err(io::Error::from)?;

    // Set the close-on-exec flag for the write ends of the pipes.
    fcntl(video_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
    fcntl(audio_write_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;

    let output_path = temp_dir_path.as_ref().join(format!("merged.{extension}"));

    let mut merge_child = merge_streams(video_read_fd, audio_read_fd, extension, &output_path)?;

    // Set the close-on-exec flag for the read ends of the pipes.
    // We use this after `merge_streams` because we want to keep the pipes open in the ffmpeg process
    fcntl(video_read_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;
    fcntl(audio_read_fd, F_SETFD(FdFlag::FD_CLOEXEC)).map_err(io::Error::from)?;

    let mut video_child = download_to_pipe(
        unsafe { OwnedFd::from_raw_fd(video_write_fd) },
        &executable_ytdl_path,
        &video_id_or_url,
        combined_format.video_format.id,
    )?;
    let mut audio_child = download_to_pipe(
        unsafe { OwnedFd::from_raw_fd(audio_write_fd) },
        executable_ytdl_path,
        video_id_or_url,
        combined_format.audio_format.id,
    )?;

    // In multithreaded context, we need to close the pipes in the parent process for all threads
    // with the close-on-exec flag set, otherwise, we may get a deadlock.
    if let Err(errno) = close(video_read_fd) {
        event!(Level::WARN, %errno, "Error closing video read pipe");
    }
    if let Err(errno) = close(audio_read_fd) {
        event!(Level::WARN, %errno, "Error closing audio read pipe");
    }

    let thumbnail_path = video
        .thumbnail
        .map(|url| get_thumbnail_path(url, video.id, temp_dir_path))
        .flatten();

    let Some(exit_code) = merge_child.wait_timeout(Duration::from_secs(timeout))? else {
        event!(Level::ERROR, "FFmpeg process timed out");

        merge_child.kill()?;
        video_child.kill()?;
        audio_child.kill()?;

        return Err(io::Error::new(io::ErrorKind::TimedOut, "FFmpeg process timed out").into());
    };

    if !exit_code.success() {
        event!(Level::ERROR, "FFmpeg exited with status `{exit_code}`");

        return Err(io::Error::new(io::ErrorKind::Other, format!("FFmpeg exited with status `{exit_code}`")).into());
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
pub fn audio_to_temp_dir(
    video: VideoInYT,
    video_id_or_url: impl AsRef<str>,
    max_file_size: u64,
    executable_ytdl_path: impl AsRef<str>,
    temp_dir_path: impl AsRef<Path>,
    timeout: u64,
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

    download_audio_to_path(
        executable_ytdl_path,
        video_id_or_url,
        audio_format.id,
        extension,
        &temp_dir_path,
        timeout,
    )?;

    event!(Level::DEBUG, "Audio downloaded");

    let thumbnail_path = get_best_thumbnail_path_in_dir(temp_dir_path)?;

    Ok(AudioInFS::new(file_path, thumbnail_path))
}
