use crate::{
    extractors::YtDlpWrapper,
    models::{CombinedFormats, Videos},
};

use bytes::{Bytes, BytesMut};
use futures::TryStreamExt as _;
use std::{
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
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
use tokio::{
    fs::DirEntry,
    sync::mpsc::{self, error::SendError},
    task::JoinHandle,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::codec::{BytesCodec, FramedRead};
use tracing::{event, field, instrument, span, Level, Span};
use youtube_dl::YoutubeDl;

const CAPACITY: usize = 64 * 1024; // 64 KiB
const REQUEST_TIMEOUT: f32 = 300.0; // 5 minutes

#[derive(Debug, thiserror::Error)]
enum SenderError {
    #[error(transparent)]
    SendToReceiver(#[from] SendError<Result<Bytes, io::Error>>),
    #[error(transparent)]
    IO(#[from] io::Error),
}

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

fn ytdl_new_with_download_args(id_or_url: &str, ytdl_path: impl AsRef<Path>, format: &str) -> YoutubeDl {
    let mut ytdl = YoutubeDl::new(id_or_url);
    ytdl.socket_timeout("15");
    ytdl.output_template("%(id)s.%(ext)s");
    ytdl.extra_arg("--no-call-home");
    ytdl.extra_arg("--no-check-certificate");
    ytdl.extra_arg("--no-cache-dir");
    ytdl.extra_arg("--no-mtime");
    ytdl.extra_arg("--no-part");
    ytdl.extra_arg("--abort-on-error");
    ytdl.youtube_dl_path(ytdl_path.as_ref());
    ytdl.format(format);

    ytdl
}

/// Get first entry from dir in loop.
/// If dir is empty, sleep for 100 ms and try again.
#[instrument(skip(path), fields(path))]
async fn get_first_entry_from_dir_in_loop(path: impl AsRef<Path>) -> Result<DirEntry, io::Error> {
    let path = path.as_ref();

    Span::current().record("path", path.display().to_string());

    let duration = Duration::from_millis(100);

    loop {
        let mut read_dir = match tokio::fs::read_dir(path).await {
            Ok(read_dir) => read_dir,
            Err(err) => {
                event!(Level::TRACE, %err, "Directory not found");

                tokio::time::sleep(duration).await;

                continue;
            }
        };

        if let Some(entry) = read_dir.next_entry().await.map_err(|err| {
            event!(Level::ERROR, "Error while getting next entry");

            err
        })? {
            return Ok(entry);
        }

        event!(Level::TRACE, "Directory is empty");

        tokio::time::sleep(duration).await;
    }
}

fn get_receiver_and_sender<T>() -> (mpsc::Sender<T>, ReceiverStream<T>) {
    let (sender, receiver) = mpsc::channel(CAPACITY);

    (sender, receiver.into())
}

pub async fn url(bot: Arc<Bot>, message: Message, YtDlpWrapper(yt_dlp_config): YtDlpWrapper) -> HandlerResult {
    // `unwrap` is safe here, because we check that `message.text` is `Some` by filters
    let url = message.text.as_ref().unwrap();
    let chat_id = message.chat_id();

    let span = span!(
        Level::DEBUG,
        "url_handler",
        message.message_id,
        chat_id,
        url,
        video_id = field::Empty,
        format_id = field::Empty,
        file_path = field::Empty
    );

    let _enter = span.enter();

    let videos = match ytdl_new_with_get_info_args(url, &yt_dlp_config.full_path).run_async().await {
        Ok(ytdl_output) => Videos::from(ytdl_output),
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
        event!(Level::ERROR, "Playlist doesn't have entries");

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
        let span = span.clone();

        let video_id = video.id.clone();
        let video_title = video.title.clone().unwrap_or("Untitled".to_owned());
        let bot = bot.clone();
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();

        handles.push(tokio::spawn(async move {
            let combined_formats = CombinedFormats::try_from(video.formats.as_ref().map(AsRef::as_ref)).map(|mut combined_formats| {
                // Filter out formats that are bigger than `max_files_size_in_bytes`
                combined_formats.skip_with_size_less_than(max_files_size_in_bytes);
                combined_formats
            });
            let Ok(Some(combined_format)) = combined_formats.as_ref().map(|combined_formats| combined_formats.first()) else {
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

            // `unwrap` is safe here, because we create `combined_formats` from `video.formats` and check that it's not empty
            let format_id = combined_format.get_format_id().unwrap();

            span.record("format_id", format_id.as_str());

            let video_downloaded = Arc::new(AtomicBool::new(false));
            let video_download_failed = Arc::new(AtomicBool::new(false));

            // Create temp dir to store videos and playlists
            let temp_dir = TempDir::new("ytdl_videos").map_err(HandlerError::new)?;

            let inner_temp_dir_path = temp_dir.path().to_path_buf();
            let inner_video_downloaded = video_downloaded.clone();
            let inner_video_download_failed = video_download_failed.clone();

            // Download video to temp dir
            let download_handle = async move {
                if ytdl_new_with_download_args(video_id.as_str(), yt_dlp_full_path, format_id.as_str())
                    .download_to_async(inner_temp_dir_path)
                    .await
                    .is_err()
                {
                    event!(Level::ERROR, "Error while downloading video");

                    inner_video_download_failed.store(true, Ordering::SeqCst);
                } else {
                    event!(Level::DEBUG, "Video downloaded successfully");

                    inner_video_downloaded.store(true, Ordering::SeqCst);
                };
            };

            let inner_video_download_failed = video_download_failed.clone();

            // We use `select!` here to avoid infinite loop to get first entry from dir in case when video download failed
            let entry = tokio::select! {
                // If this branch is executed, it means that video is downloaded successfully earlier than entry has been got from dir
                // or video download failed.
                // We use `tokio::spawn` to possible continue to download video in background even if we got entry from dir.
                result = tokio::spawn(download_handle) => {
                    result.map_err(HandlerError::new)?;

                    // If video download failed, we don't need to get first entry from dir, so we continue to the next combined format
                    if inner_video_download_failed.load(Ordering::SeqCst) {
                        return Ok(EventReturn::Finish);
                    }

                    // If video is downloaded successfully, we need to get first entry from dir
                    get_first_entry_from_dir_in_loop(temp_dir.as_ref()).await
                }
                // If this branch is executed, it means that dir and file are created, but video is not downloaded yet
                result = get_first_entry_from_dir_in_loop(temp_dir.as_ref()) => result
            }
            .map_err(HandlerError::new)?;

            event!(Level::DEBUG, "Got entry from temp dir");

            let (sender, receiver) = get_receiver_and_sender();

            // Read file and send bytes to `receiver` until video is downloaded or video download failed
            let sender_handle = async move {
                let file_path = entry.path();

                span.record("file_path", file_path.display().to_string());

                let duration = Duration::from_millis(100);

                let mut file = tokio::fs::File::open(file_path).await?;

                while !video_downloaded.load(Ordering::SeqCst) && !video_download_failed.load(Ordering::SeqCst) {
                    for bytes in FramedRead::with_capacity(&mut file, BytesCodec::new(), CAPACITY)
                        .map_ok(BytesMut::freeze)
                        .try_collect::<Vec<_>>()
                        .await?
                    {
                        event!(Level::TRACE, bytes_len = %bytes.len(), "Sending bytes");

                        sender.send(Ok(bytes)).await?;
                    }

                    tokio::time::sleep(duration).await;
                }

                Ok::<_, SenderError>(())
            };

            // Read bytes from `sender` and send them to Telegram until video is downloaded or video download failed
            // or request timeout is reached
            let receiver_handle = async move {
                let video_thumnail_input_file = video.get_best_thumbnail_url().map(InputFile::url);

                bot.send_with_timeout(
                    SendVideo::new(chat_id, InputFile::stream(receiver))
                        .reply_to_message_id(message.message_id)
                        .allow_sending_without_reply(true)
                        .supports_streaming(true)
                        .thumbnail_option(video_thumnail_input_file),
                    REQUEST_TIMEOUT,
                )
                .await
            };

            tokio::select! {
                result = sender_handle => match result {
                    Ok(()) => Ok(EventReturn::Finish),
                    Err(err) => {
                        event!(Level::ERROR, %err, "Error while sending bytes");

                        Err(HandlerError::new(err))
                    }
                },
                result = tokio::spawn(receiver_handle) => match result {
                    Ok(Ok(_message)) => Ok(EventReturn::Finish),
                    Ok(Err(err)) => {
                        event!(Level::ERROR, %err, "Error while sending video");

                        Err(HandlerError::new(err))
                    }
                    Err(err) => {
                        event!(Level::ERROR, %err, "Error while joining `receiver_handle`");

                        Err(HandlerError::new(err))
                    }
                }
            }
        }));
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(event_return)) => match event_return {
                EventReturn::Finish => continue,
                _ => unreachable!(),
            },
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
