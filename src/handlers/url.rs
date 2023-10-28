use crate::{
    extractors::YtDlpWrapper,
    models::{CombinedFormats, Videos},
};

use std::{
    path::Path,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc,
    },
    thread,
};
use telers::{
    enums::ParseMode,
    errors::HandlerError,
    event::{telegram::HandlerResult, EventReturn},
    methods::{SendMessage, SendVideo},
    types::{InputFile, Message},
    utils::text_decorations::{TextDecoration, HTML_DECORATION},
    Bot,
};
use tempdir::TempDir;
use tokio::task::JoinHandle;
use tracing::{event, Level};
use youtube_dl::YoutubeDl;

const MEGABYTE: u64 = 1_000_000;

fn ytdl_new_with_get_info_args(url: &str, ytdl_path: &str) -> YoutubeDl {
    let mut ytdl = YoutubeDl::new(url);
    ytdl.socket_timeout("15");
    ytdl.extra_arg("--no-call-home");
    ytdl.extra_arg("--no-check-certificate");
    ytdl.extra_arg("--skip-download");
    ytdl.extra_arg("--abort-on-error");
    ytdl.youtube_dl_path(ytdl_path);

    ytdl
}

fn ytdl_new_with_download_args(id_or_url: &str, ytdl_path: &str, format: &str) -> YoutubeDl {
    let mut ytdl = YoutubeDl::new(id_or_url);
    ytdl.socket_timeout("15");
    ytdl.output_template("%(id)s");
    ytdl.extra_arg("--no-call-home");
    ytdl.extra_arg("--no-check-certificate");
    ytdl.extra_arg("--no-cache-dir");
    ytdl.extra_arg("--no-mtime");
    ytdl.extra_arg("--abort-on-error");
    ytdl.youtube_dl_path(ytdl_path);
    ytdl.format(format);

    ytdl
}

async fn await_handles(handles: Vec<JoinHandle<HandlerResult>>) -> Result<(), HandlerError> {
    for handle in handles {
        match handle.await {
            Ok(result) => result?,
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");

                return Err(HandlerError::new(err));
            }
        };
    }

    Ok(())
}

fn spawn_delete_file(path: impl AsRef<Path>) {
    let path = path.as_ref().to_owned();

    thread::spawn(move || {
        if let Err(err) = std::fs::remove_file(path.as_path()) {
            event!(Level::ERROR, %err, ?path, "Error while deleting file");
        }
    });
}

pub async fn url(bot: Arc<Bot>, message: Message, YtDlpWrapper(yt_dlp_config): YtDlpWrapper) -> HandlerResult {
    // `unwrap` is safe here, because we check that `message.text` is `Some` by filters
    let url = message.text.as_ref().unwrap();
    let chat_id = message.chat_id();

    let videos = match ytdl_new_with_get_info_args(url, &yt_dlp_config.full_path).run_async().await {
        Ok(ytdl_output) => Videos::from(ytdl_output),
        Err(err) => {
            event!(Level::ERROR, %err, url, "Error while getting video/playlist info");

            bot.send(
                &SendMessage::new(
                    chat_id,
                    "Sorry, an error occurred while getting video/playlist info. Try again later.",
                )
                .reply_to_message_id(message.message_id)
                .allow_sending_without_reply(true),
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    if videos.is_empty() {
        event!(Level::ERROR, url, "Playlist doesn't have entries");

        bot.send(
            &SendMessage::new(chat_id, "Playlist doesn't have videos to download.")
                .reply_to_message_id(message.message_id)
                .allow_sending_without_reply(true),
        )
        .await?;

        return Ok(EventReturn::Finish);
    }

    // Max files size to download in one request.
    // We use this value to choose the best format for each video and avoid errors from Telegram.
    let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
    let max_files_size_in_mb = max_files_size_in_bytes / MEGABYTE;

    let count_downloaded_videos = Arc::new(AtomicU16::new(0));
    let count_videos_skipped_by_size = Arc::new(AtomicU16::new(0));

    let mut handles: Vec<JoinHandle<HandlerResult>> = vec![];

    for video in videos {
        // Create temp dir to store videos and playlists
        let temp_dir = TempDir::new("ytdl_videos").map_err(HandlerError::new)?;

        let video_id = video.id.clone();
        let video_title = video.title.clone().unwrap_or("Untitled".to_owned());
        let bot = bot.clone();
        let yt_dlp_config = yt_dlp_config.clone();
        let count_downloaded_videos = count_downloaded_videos.clone();
        let count_videos_skipped_by_size = count_videos_skipped_by_size.clone();

        handles.push(tokio::spawn(async move {
            let temp_dir_path = temp_dir.path();
            let combined_formats = CombinedFormats::try_from(video.formats.as_ref().map(AsRef::as_ref)).map(|mut combined_formats| {
                // Filter out formats that are bigger than `max_files_size_in_bytes`
                combined_formats.skip_with_size_less_than(max_files_size_in_bytes);
                combined_formats
            });
            let combined_formats_is_err_or_empty = combined_formats.as_ref().map_or(true, CombinedFormats::is_empty);

            if combined_formats_is_err_or_empty {
                event!(Level::ERROR, ?video, "No combined formats found");

                bot.send(
                    &SendMessage::new(
                        chat_id,
                        format!(
                            "Sorry, suitable formats for video {title} not found. \
                            Maybe video size is too big or video has unsupported format.",
                            title = HTML_DECORATION.code(HTML_DECORATION.quote(video_title.as_str()).as_str()),
                        ),
                    )
                    .parse_mode(ParseMode::HTML),
                )
                .await?;

                return Ok(EventReturn::Finish);
            }

            // `unwrap` is safe here, because we check that `combined_formats` is `Ok` and not empty
            let combined_formats = combined_formats.unwrap();

            for combined_format in combined_formats {
                // `unwrap` is safe here, because we create `combined_formats` from `video.formats` and check that it's not empty
                let format_id = combined_format.get_format_id().unwrap();

                if let Err(err) = ytdl_new_with_download_args(video_id.as_str(), yt_dlp_config.full_path.as_str(), format_id.as_str())
                    .download_to_async(temp_dir_path)
                    .await
                {
                    event!(Level::ERROR, %err, ?video, ?combined_format, "Error while downloading video");

                    continue;
                };

                count_downloaded_videos.fetch_add(1, Ordering::SeqCst);

                let mut stream = tokio::fs::read_dir(temp_dir_path).await.map_err(|err| {
                    event!(Level::ERROR, %err, ?temp_dir_path, "Error while reading temp dir");

                    HandlerError::new(err)
                })?;
                let entry = stream
                    .next_entry()
                    .await
                    .map_err(|err| {
                        event!(Level::ERROR, %err, ?combined_format, ?temp_dir_path, "Error while reading temp dir");

                        HandlerError::new(err)
                    })?
                    .expect("Temp dir is empty, but it should contain at least one file");

                let file_path = entry.path();
                let file_path_ref = file_path.as_path();

                let input_file = InputFile::fs(file_path_ref);

                let metadata = match tokio::fs::metadata(file_path_ref).await {
                    Ok(metadata) => metadata,
                    Err(err) => {
                        event!(Level::ERROR, %err, file_path = ?file_path_ref, "Error while reading file metadata");

                        spawn_delete_file(file_path_ref);

                        bot.send(
                            &SendMessage::new(
                                chat_id,
                                "Sorry, something went wrong while reading file metadata. File skipped to avoid errors.",
                            )
                            .reply_to_message_id(message.message_id)
                            .allow_sending_without_reply(true),
                        )
                        .await?;

                        return Err(HandlerError::new(err));
                    }
                };

                let file_size_in_bytes = metadata.len();

                // We check before downloading, but we need to check again, maybe real file size is bigger than approximated?
                // I don't sure that it's possible.
                if file_size_in_bytes > max_files_size_in_bytes {
                    let file_size_in_mb = file_size_in_bytes / MEGABYTE;

                    event!(
                        Level::WARN,
                        path = ?file_path,
                        file_size_in_bytes,
                        file_size_in_mb,
                        "File size is too big, skipping"
                    );

                    spawn_delete_file(file_path_ref);

                    count_videos_skipped_by_size.fetch_add(1, Ordering::SeqCst);

                    bot.send(&SendMessage::new(
                        chat_id,
                        format!(
                            "Sorry, file size is too big to send. \
                            Max file size is {max_files_size_in_mb} MB. \
                            Current file size is {file_size_in_mb} MB. \
                            File skipped to avoid errors.",
                        ),
                    ))
                    .await?;

                    continue;
                }

                bot.send(
                    SendVideo::new(chat_id, input_file)
                        .reply_to_message_id(message.message_id)
                        .allow_sending_without_reply(true)
                        .supports_streaming(true)
                        .thumbnail_option(video.get_best_thumbnail().map(|thumbnail| {
                            InputFile::url(thumbnail.url.as_ref().expect("Thumbnail URL is `None`, but it should be `Some`"))
                        })),
                )
                .await?;

                return Ok(EventReturn::Finish);
            }

            bot.send(
                &SendMessage::new(
                    chat_id,
                    format!(
                        "Sorry, an error occurred while downloading video {title}. Try again later.",
                        title = HTML_DECORATION.code(HTML_DECORATION.quote(video_title.as_str()).as_str()),
                    ),
                )
                .reply_to_message_id(message.message_id)
                .allow_sending_without_reply(true)
                .parse_mode(ParseMode::HTML),
            )
            .await?;

            Ok(EventReturn::Finish)
        }));
    }

    await_handles(handles).await?;

    if count_downloaded_videos.load(Ordering::SeqCst) == 0 {
        // If we skip some videos because of their size, we don't need to send a message about download failure again
        if count_videos_skipped_by_size.load(Ordering::SeqCst) > 0 {
            return Ok(EventReturn::Finish);
        }

        event!(Level::ERROR, url, "No videos found in playlist");

        bot.send(
            &SendMessage::new(
                chat_id,
                "Sorry, something went wrong. Not any video has been downloaded. Try again later.",
            )
            .reply_to_message_id(message.message_id)
            .allow_sending_without_reply(true),
        )
        .await?;
    }

    Ok(EventReturn::Finish)
}
