use crate::{
    cmd::ytdl,
    errors::DownloadError,
    fs::get_best_thumbnail_path_in_dir,
    models::{AudioInFS, VideoInFS, VideoInYT},
};

use tempfile::TempDir;
use tracing::{event, field, instrument, Level, Span};

#[instrument(skip_all, fields(video = video.id, format_id = field::Empty, file_path = field::Empty))]
pub async fn video_to_temp_dir(
    video: VideoInYT,
    temp_dir: &TempDir,
    max_files_size_in_bytes: u64,
    executable_ytdl_path: &str,
    allow_playlist: bool,
    download_thumbnails: bool,
) -> Result<VideoInFS, DownloadError> {
    let video_id = video.id.clone();

    let mut combined_formats = video.get_combined_formats();
    combined_formats.sort_by_priority_and_skip_by_size(max_files_size_in_bytes);

    let Some(combined_format) = combined_formats.first().cloned() else {
        event!(Level::ERROR, video_id, "No format found for video");

        return Err(DownloadError::NoFormatFound {
            video_id: video_id.into_boxed_str(),
        });
    };

    drop(combined_formats);

    let extension = combined_format.get_extension();

    Span::current().record("format_id", &combined_format.format_id());

    event!(Level::DEBUG, %combined_format, "Got combined format");

    let file_path = temp_dir.path().join(format!("{video_id}.{extension}"));

    Span::current().record("file_path", file_path.display().to_string());

    event!(Level::DEBUG, ?file_path, "Got file path");

    ytdl::download_video_to_path(
        executable_ytdl_path,
        temp_dir.path().to_string_lossy().as_ref(),
        video_id.as_str(),
        combined_format.format_id().as_ref(),
        extension,
        allow_playlist,
        download_thumbnails,
    )
    .await?;

    event!(Level::DEBUG, "video downloaded");

    let thumbnail_path = if download_thumbnails {
        get_best_thumbnail_path_in_dir(temp_dir.path(), video_id.as_str())
            .await
            .map_err(DownloadError::ThumbnailPathFailed)?
    } else {
        None
    };

    event!(Level::TRACE, ?thumbnail_path, "Got thumbnail path");

    Ok(VideoInFS::new(file_path, thumbnail_path))
}

#[instrument(skip_all, fields(video = video.id, format_id = field::Empty, file_path = field::Empty))]
pub async fn audio_to_temp_dir(
    video: VideoInYT,
    temp_dir: &TempDir,
    max_files_size_in_bytes: u64,
    executable_ytdl_path: &str,
    download_thumbnails: bool,
) -> Result<AudioInFS, DownloadError> {
    let video_id = video.id.clone();

    let mut audio_formats = video.get_audio_formats();
    audio_formats.sort_by_priority_and_skip_by_size(max_files_size_in_bytes);

    let Some(audio_format) = audio_formats.first().cloned() else {
        event!(Level::ERROR, video_id, "No format found for audio");

        return Err(DownloadError::NoFormatFound {
            video_id: video_id.into_boxed_str(),
        });
    };

    drop(audio_formats);

    let extension = audio_format.codec.get_extension();

    Span::current().record("format_id", audio_format.id);

    event!(Level::DEBUG, %audio_format, "Got audio format");

    let file_path = temp_dir.path().join(format!("{video_id}.{extension}"));

    Span::current().record("file_path", file_path.display().to_string());

    event!(Level::DEBUG, ?file_path, "Got file path");

    ytdl::download_audio_to_path(
        executable_ytdl_path,
        temp_dir.path().to_string_lossy().as_ref(),
        video_id.as_str(),
        audio_format.id,
        extension,
        download_thumbnails,
    )
    .await?;

    event!(Level::DEBUG, "Audio downloaded");

    let thumbnail_path = if download_thumbnails {
        get_best_thumbnail_path_in_dir(temp_dir.path(), video_id.as_str())
            .await
            .map_err(DownloadError::ThumbnailPathFailed)?
    } else {
        None
    };

    Ok(AudioInFS::new(file_path, thumbnail_path))
}
