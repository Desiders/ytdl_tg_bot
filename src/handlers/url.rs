use crate::{cmd::ytdl, extractors::YtDlpWrapper};

use std::{io, path::Path, sync::Arc, time::Duration};
use telers::{
    enums::ParseMode,
    errors::HandlerError,
    event::{telegram::HandlerResult, EventReturn},
    methods::{SendMessage, SendVideo},
    types::{InputFile, Message},
    utils::text_decorations::{TextDecoration, HTML_DECORATION},
    Bot,
};
use tempfile::tempdir;
use tokio::{fs::DirEntry, task::JoinHandle};
use tracing::{event, field, instrument, span, Level, Span};

const REQUEST_TIMEOUT: f32 = 300.0; // 5 minutes

/// Get entry from dir in loop.
/// If dir is empty, sleep for 100 ms and try again.
#[instrument(skip(path), fields(path))]
async fn get_entry_from_dir_in_loop(path: impl AsRef<Path>, filename: &str) -> Result<DirEntry, io::Error> {
    let path = path.as_ref();

    Span::current().record("path", path.display().to_string());

    let duration = Duration::from_millis(250);

    loop {
        tokio::time::sleep(duration).await;

        let mut read_dir = match tokio::fs::read_dir(path).await {
            Ok(read_dir) => read_dir,
            Err(err) => {
                event!(Level::TRACE, %err, "Directory not found");

                continue;
            }
        };

        if let Some(entry) = read_dir.next_entry().await.map_err(|err| {
            event!(Level::ERROR, "Error while getting next entry");

            err
        })? {
            if entry.file_name() != filename {
                event!(Level::TRACE, "Entry is not video file");

                continue;
            }

            return Ok(entry);
        }

        event!(Level::TRACE, "Directory is empty");

        tokio::time::sleep(duration).await;
    }
}

pub async fn url(bot: Arc<Bot>, message: Message, YtDlpWrapper(yt_dlp_config): YtDlpWrapper) -> HandlerResult {
    // `unwrap` is safe here, because we check that `message.text` is `Some` by filters
    let url = message.text.as_ref().unwrap();
    let chat_id = message.chat_id();

    let span = span!(Level::DEBUG, "url_handler", message.message_id, chat_id, url);

    let _enter = span.enter();

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

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
        event!(Level::ERROR, "Playlist doesn't have videos");

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

    let mut handles: Vec<JoinHandle<HandlerResult>> = vec![];

    for video in videos {
        let span = span!(
            parent: &span,
            Level::DEBUG,
            "video_downloader",
            video.id,
            format_id = field::Empty,
            file_path = field::Empty
        );

        let temp_dir = tempdir().map_err(HandlerError::new)?;

        let video_id = video.id.clone();
        let video_title = video.title.clone().unwrap_or("Untitled".to_owned());
        let bot = bot.clone();
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();

        handles.push(tokio::spawn(async move {
            let _enter = span.enter();

            let mut combined_formats = video.get_combined_formats();
            // Filter out formats that are bigger than `max_files_size_in_bytes`
            combined_formats.skip_with_size_less_than(max_files_size_in_bytes);
            combined_formats.sort_by_format_id_priority();

            let Some(combined_format) = combined_formats.last() else {
                event!(Level::ERROR, "No combined formats found");

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
            };

            event!(Level::DEBUG, ?combined_format, "Got combined format");

            let format_id = combined_format.format_id();
            let format_extension = combined_format.get_extension();
            let temp_dir_path = temp_dir.path();
            let file_path = temp_dir_path.join(format!("{video_id}.{format_extension}"));

            span.record("format_id", format_id);
            span.record("file_path", file_path.display().to_string());

            match ytdl::download_video_to_path(
                yt_dlp_full_path.as_str(),
                temp_dir_path.to_string_lossy().as_ref(),
                video_id.as_str(),
                format_id,
                format_extension,
            )
            .await
            {
                Ok(()) => {
                    event!(Level::DEBUG, "Video downloading finished");
                }
                Err(err) => {
                    event!(Level::ERROR, %err, "Error while downloading video");

                    return Err(HandlerError::new(err));
                }
            }

            let _message = bot
                .send_with_timeout(
                    SendVideo::new(chat_id, InputFile::fs(file_path))
                        .reply_to_message_id(message.message_id)
                        .allow_sending_without_reply(true)
                        .supports_streaming(true)
                        .thumbnail_option(video.get_best_thumbnail_url().map(InputFile::url)),
                    REQUEST_TIMEOUT,
                )
                .await?;

            Ok(EventReturn::Finish)
        }));
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(_)) => continue,
            Ok(Err(err)) => {
                event!(Level::ERROR, %err, "Error while sending video");

                bot.send(
                    &SendMessage::new(chat_id, "Sorry, an error occurred while sending video. Try again later.")
                        .reply_to_message_id(message.message_id)
                        .allow_sending_without_reply(true),
                )
                .await?;

                return Err(err);
            }
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");

                bot.send(
                    &SendMessage::new(chat_id, "Sorry, an error occurred while sending video. Try again later.")
                        .reply_to_message_id(message.message_id)
                        .allow_sending_without_reply(true),
                )
                .await?;

                return Err(HandlerError::new(err));
            }
        }
    }

    event!(Level::DEBUG, "All handles finished");

    Ok(EventReturn::Finish)
}
