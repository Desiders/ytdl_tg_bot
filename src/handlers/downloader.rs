use crate::{
    cmd::ytdl,
    config::PhantomVideoId,
    extractors::{BotConfigWrapper, YtDlpWrapper},
    models::{CombinedFormat, CombinedFormats},
};

use futures_util::{TryFutureExt as _, TryStreamExt as _};
use std::{
    fs::Metadata,
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use telers::{
    enums::ParseMode,
    errors::{HandlerError, SessionErrorKind},
    event::{telegram::HandlerResult, EventReturn},
    methods::{
        AnswerInlineQuery, DeleteMessage, EditMessageCaption, EditMessageMedia, EditMessageReplyMarkup, SendChatAction, SendMessage,
        SendVideo,
    },
    types::{
        input_file::DEFAULT_CAPACITY, ChosenInlineResult, InlineKeyboardButton, InlineKeyboardMarkup, InlineQuery, InlineQueryResult,
        InlineQueryResultArticle, InlineQueryResultCachedVideo, InputFile, InputMediaVideo, InputTextMessageContent, Message,
    },
    utils::text_decorations::{TextDecoration, HTML_DECORATION},
    Bot, Context,
};
use tempfile::tempdir;
use tokio::task::JoinHandle;
use tokio_util::codec::{BytesCodec, FramedRead};
use tracing::{event, field, instrument, span, Level, Span};
use uuid::Uuid;

const REQUEST_TIMEOUT: f32 = 60.0 * 5.0; // 5 minutes
const VIDEOS_IN_PLAYLIST_CACHE_TIME: i32 = 60 * 60; // 1 hour

const MAX_THUMBNAIL_SIZE_IN_BYTES: u64 = 1024 * 200; // 200 KB
const ACCEPTABLE_THUMBNAIL_EXTENSIONS: [&str; 2] = ["jpg", "jpeg"];

fn filter_and_get_combined_format<'a>(
    formats: &'a mut CombinedFormats<'a>,
    max_files_size_in_bytes: u64,
) -> Option<&'a CombinedFormat<'a>> {
    formats.skip_with_size_less_than(max_files_size_in_bytes);
    formats.sort_by_format_id_priority();

    formats.first()
}

async fn get_best_thumbnail_path_in_dir(path_dir: impl AsRef<Path>, name: &str) -> Result<Option<PathBuf>, io::Error> {
    let path_dir = path_dir.as_ref();

    let mut read_dir = tokio::fs::read_dir(path_dir).await?;

    let mut best_thumbnail: Option<(PathBuf, Metadata)> = None;

    while let Some(entry) = read_dir.next_entry().await? {
        let entry_name = entry.file_name();

        // If names are equal, then it's video file, not thumbnail
        if entry_name == name {
            continue;
        }

        let path = entry.path();

        let Some(entry_extension) = path.extension() else {
            continue;
        };

        if !ACCEPTABLE_THUMBNAIL_EXTENSIONS.contains(&entry_extension.to_str().unwrap_or_default()) {
            continue;
        }

        let entry_metadata = entry.metadata().await?;
        let entry_size = entry_metadata.len();

        if entry_size > MAX_THUMBNAIL_SIZE_IN_BYTES {
            continue;
        }

        if let Some((_, best_thumbnail_metadata)) = best_thumbnail.as_ref() {
            if entry_size > best_thumbnail_metadata.len() {
                event!(Level::TRACE, path = ?entry.path(), "Got better thumbnail");

                best_thumbnail = Some((path, entry_metadata));
            }
        } else {
            event!(Level::TRACE, path = ?entry.path(), "Got first thumbnail");

            best_thumbnail = Some((entry.path(), entry.metadata().await?));
        }
    }

    Ok(best_thumbnail.map(|(path, _)| path))
}

async fn send_upload_action_in_loop(bot: &Bot, chat_id: i64) {
    loop {
        if let Err(err) = bot.send(SendChatAction::new(chat_id, "upload_video")).await {
            event!(Level::ERROR, %err, "Error while sending upload action");

            break;
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn error_occured(
    bot: &Bot,
    chat_id: i64,
    reply_to_message_id: i64,
    text: &str,
    parse_mode: Option<ParseMode>,
) -> Result<Message, SessionErrorKind> {
    bot.send(
        SendMessage::new(chat_id, text)
            .reply_to_message_id(reply_to_message_id)
            .allow_sending_without_reply(true)
            .parse_mode_option(parse_mode),
    )
    .await
}

#[instrument(skip_all, fields(message_id, chat_id = chat.id, url))]
pub async fn video_download(
    bot: Arc<Bot>,
    context: Arc<Context>,
    Message {
        message_id,
        text: url,
        chat,
        ..
    }: Message,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
) -> HandlerResult {
    let url = if let Some(url) = context.get("video_url") {
        url.downcast_ref::<Box<str>>().expect("Url should be `Box<str>`").clone()
    } else {
        event!(Level::WARN, "Url not found in context. `text_contains_url` filter should do this");

        url.unwrap()
    };
    let chat_id = chat.id;

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            error_occured(
                &bot,
                chat_id,
                message_id,
                "Sorry, an error occurred while getting video/playlist info. Try again later.",
                None,
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    if videos.is_empty() {
        event!(Level::WARN, "Playlist doesn't have videos");

        error_occured(&bot, chat_id, message_id, "Playlist doesn't have videos.", None).await?;

        return Ok(EventReturn::Finish);
    }

    let mut handles: Vec<JoinHandle<HandlerResult>> = vec![];

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { send_upload_action_in_loop(&bot, chat_id).await }
    });

    for video in videos {
        let span = span!(
            parent: Span::current(),
            Level::DEBUG,
            "video_downloader",
            video_id = video.id, format_id = field::Empty, file_path = field::Empty,
        );

        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();

            HandlerError::new(err)
        })?;

        let video_id = video.id.clone();
        let video_title = video.title.clone().unwrap_or("Untitled".to_owned());
        let bot = bot.clone();
        let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();

        handles.push(tokio::spawn(async move {
            let _enter = span.enter();

            let mut combined_formats = video.get_combined_formats();

            event!(Level::TRACE, ?combined_formats, "Got combined formats");

            let Some(combined_format) = filter_and_get_combined_format(&mut combined_formats, max_files_size_in_bytes) else {
                event!(Level::ERROR, "No combined formats found");

                error_occured(
                    &bot,
                    chat_id,
                    message_id,
                    format!(
                        "Sorry, suitable formats for video {title} not found. \
                        Maybe video size is too big or video has unsupported format.",
                        title = HTML_DECORATION.code(HTML_DECORATION.quote(video_title.as_str()).as_str()),
                    )
                    .as_str(),
                    Some(ParseMode::HTML),
                )
                .await?;

                return Ok(EventReturn::Finish);
            };

            let file_path = temp_dir.path().join(format!(
                "{video_id}.{format_extension}",
                format_extension = combined_format.get_extension()
            ));

            span.record("format_id", combined_format.format_id());
            span.record("file_path", file_path.display().to_string());

            event!(Level::DEBUG, ?combined_format, "Got combined format");

            match ytdl::download_video_to_path(
                yt_dlp_full_path.as_str(),
                temp_dir.path().to_string_lossy().as_ref(),
                video_id.as_str(),
                combined_format.format_id().as_ref(),
                combined_format.get_extension(),
                false,
                true,
            )
            .await
            {
                Ok(()) => {
                    event!(Level::DEBUG, "Video and audio downloading finished");
                }
                Err(err) => {
                    event!(Level::ERROR, %err, "Error while downloading video and audio");

                    return Err(HandlerError::new(err));
                }
            }

            let thumbnail_input_file = get_best_thumbnail_path_in_dir(temp_dir.path(), video_id.as_str())
                .await
                .ok()
                .and_then(|thumbnail_path| thumbnail_path.map(InputFile::fs));

            #[allow(clippy::cast_possible_truncation)]
            bot.send_with_timeout(
                SendVideo::new(chat_id, InputFile::fs(file_path))
                    .reply_to_message_id(message_id)
                    .allow_sending_without_reply(true)
                    .width_option(video.width)
                    .height_option(video.height)
                    .duration_option(video.duration.map(|duration| duration as i64))
                    .thumbnail_option(thumbnail_input_file)
                    .supports_streaming(true),
                REQUEST_TIMEOUT,
            )
            .await?;

            Ok(EventReturn::Finish)
        }));
    }

    for handle in handles {
        let error_occured = error_occured(
            &bot,
            chat_id,
            message_id,
            "Sorry, an error occurred while sending video. Try again later.",
            None,
        );

        match handle.await {
            Ok(Ok(_)) => continue,
            Ok(Err(err)) => {
                event!(Level::ERROR, %err, "Error while sending video");

                upload_action_task.abort();

                error_occured.await?;

                return Err(err);
            }
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");

                upload_action_task.abort();

                error_occured.await?;

                return Err(HandlerError::new(err));
            }
        }
    }

    upload_action_task.abort();

    event!(Level::DEBUG, "All handles finished");

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, filesize))]
async fn progress_by_edit_message(
    bot: &Bot,
    inline_message_id: &str,
    text_template: &str,
    filesize_progress: Arc<AtomicUsize>,
    filesize: f64,
) -> Result<(), SessionErrorKind> {
    let mut last_progress = 0;

    loop {
        let filesize_progress = filesize_progress.load(Ordering::SeqCst);

        #[allow(clippy::cast_precision_loss)]
        #[allow(clippy::cast_sign_loss)]
        #[allow(clippy::cast_possible_truncation)]
        let progress = (filesize_progress as f64 / filesize * 100.0).round() as usize;

        if progress == last_progress {
            tokio::time::sleep(Duration::from_millis(50)).await;

            continue;
        }

        last_progress = progress;

        if progress > 95 {
            bot.send(
                EditMessageReplyMarkup::new()
                    .inline_message_id(inline_message_id)
                    .reply_markup(InlineKeyboardMarkup::new([[InlineKeyboardButton::new(format!(
                        "{text_template} 100%"
                    ))
                    .callback_data("video_progress")]])),
            )
            .await?;

            break;
        }

        bot.send(
            EditMessageReplyMarkup::new()
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[InlineKeyboardButton::new(format!(
                    "{text_template} {progress}%"
                ))
                .callback_data("video_progress")]])),
        )
        .await?;

        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    Ok(())
}

async fn error_chosen_inline_result(
    bot: &Bot,
    text: &str,
    inline_message_id: &str,
    parse_mode: Option<ParseMode>,
) -> Result<(), SessionErrorKind> {
    bot.send(
        EditMessageCaption::new(text)
            .inline_message_id(inline_message_id)
            .parse_mode_option(parse_mode),
    )
    .await
    .map(|_| ())
}

#[allow(clippy::module_name_repetitions)]
#[instrument(skip_all, fields(inline_message_id, url, video_id = field::Empty, format_id = field::Empty, file_path = field::Empty))]
pub async fn video_download_chosen_inline_result(
    bot: Arc<Bot>,
    ChosenInlineResult {
        inline_message_id,
        query: url,
        ..
    }: ChosenInlineResult,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    BotConfigWrapper(bot_config): BotConfigWrapper,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), false).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            error_chosen_inline_result(
                &bot,
                "Sorry, an error occurred while getting video/playlist info. Try again later.",
                inline_message_id,
                None,
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    if videos.is_playlist() {
        event!(Level::WARN, "Got playlist instead of video. This should not happen");
    }

    let Some(video) = videos.front().cloned() else {
        event!(Level::ERROR, "Video not found");

        error_chosen_inline_result(&bot, "Sorry, video not found.", inline_message_id, None).await?;

        return Ok(EventReturn::Finish);
    };

    drop(videos);

    Span::current().record("video_id", video.id.as_str());

    let mut combined_formats = video.get_combined_formats();

    event!(Level::TRACE, ?combined_formats, "Got combined formats");

    let Some(combined_format) = filter_and_get_combined_format(&mut combined_formats, yt_dlp_config.max_files_size_in_bytes) else {
        event!(Level::ERROR, "No combined formats found");

        let video_title = video.title.as_deref().unwrap_or("Untitled");

        error_chosen_inline_result(
            &bot,
            format!(
                "Sorry, suitable formats for video {title} not found. \
                Maybe video size is too big or video has unsupported format.",
                title = HTML_DECORATION.code(HTML_DECORATION.quote(video_title).as_str()),
            )
            .as_str(),
            inline_message_id,
            Some(ParseMode::HTML),
        )
        .await?;

        return Ok(EventReturn::Finish);
    };

    event!(Level::DEBUG, ?combined_format, "Got combined format");

    let temp_dir = tempdir().map_err(HandlerError::new)?;

    let file_path = temp_dir.path().join(format!(
        "{video_id}.{format_extension}",
        video_id = video.id.as_str(),
        format_extension = combined_format.get_extension(),
    ));

    Span::current().record("format_id", combined_format.format_id());
    Span::current().record("file_path", file_path.display().to_string());

    event!(Level::DEBUG, "Downloading video and audio");

    match ytdl::download_video_to_path(
        yt_dlp_config.full_path.as_str(),
        temp_dir.path().to_string_lossy().as_ref(),
        video.id.as_str(),
        combined_format.format_id().as_ref(),
        combined_format.get_extension(),
        false,
        true,
    )
    .await
    {
        Ok(()) => {
            event!(Level::DEBUG, "Video and audio downloading finished");
        }
        Err(err) => {
            event!(Level::ERROR, %err, "Error while downloading video and audio");

            return Err(HandlerError::new(err));
        }
    }

    let filesize_progress = Arc::new(AtomicUsize::new(0));

    let thumbnail_input_file = get_best_thumbnail_path_in_dir(temp_dir.path(), video.id.as_str())
        .await
        .ok()
        .and_then(|thumbnail_path| thumbnail_path.map(InputFile::fs));

    if let Some(filesize) = combined_format.filesize_or_approx() {
        let bot = bot.clone();
        let inline_message_id = inline_message_id.to_owned();
        let filesize_progress = filesize_progress.clone();

        tokio::spawn(async move {
            progress_by_edit_message(&bot, inline_message_id.as_str(), "Sending video...", filesize_progress, filesize).await
        });
    }

    let Message {
        video, chat, message_id, ..
    } = bot
        .send_with_timeout(
            SendVideo::new(
                bot_config.receiver_video_chat_id,
                InputFile::stream(Box::pin(
                    tokio::fs::File::open(file_path)
                        .map_ok(move |file| {
                            FramedRead::with_capacity(file, BytesCodec::new(), DEFAULT_CAPACITY).map_ok(move |bytes_mut| {
                                let bytes = bytes_mut.freeze();

                                filesize_progress.fetch_add(bytes.len(), Ordering::SeqCst);

                                bytes
                            })
                        })
                        .try_flatten_stream(),
                )),
            )
            .thumbnail_option(thumbnail_input_file)
            .supports_streaming(true)
            .disable_notification(true),
            REQUEST_TIMEOUT,
        )
        .await?;

    tokio::spawn({
        let bot = bot.clone();

        async move {
            if let Err(err) = bot.send(DeleteMessage::new(chat.id, message_id)).await {
                event!(Level::ERROR, %err, "Error while deleting video");
            }
        }
    });

    bot.send_with_timeout(
        // `unwrap` is safe here, because `video` is always `Some` in `SendVideo` response
        EditMessageMedia::new(InputMediaVideo::new(InputFile::id(video.unwrap().file_id.as_ref())))
            .inline_message_id(inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
        REQUEST_TIMEOUT,
    )
    .await?;

    Ok(EventReturn::Finish)
}

async fn error_inline_query_occured(bot: &Bot, query_id: &str, text: &str) -> Result<(), SessionErrorKind> {
    let result = InlineQueryResultArticle::new(query_id, text, InputTextMessageContent::new(text));
    let results = [result];

    bot.send(AnswerInlineQuery::new(query_id, results)).await.map(|_| ())
}

#[allow(clippy::module_name_repetitions)]
#[instrument(skip_all, fields(query_id, url))]
pub async fn video_select_inline_query(
    bot: Arc<Bot>,
    InlineQuery {
        id: query_id, query: url, ..
    }: InlineQuery,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    PhantomVideoId(phantom_video_id): PhantomVideoId,
) -> HandlerResult {
    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            error_inline_query_occured(
                &bot,
                query_id.as_ref(),
                "Sorry, an error occurred while getting video/playlist info. Try again later.",
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    if videos.is_empty() {
        event!(Level::WARN, "Playlist doesn't have videos");

        error_inline_query_occured(&bot, query_id.as_ref(), "Playlist doesn't have videos.").await?;

        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos.len());

    for video in videos {
        let video_title = video.title.as_deref().unwrap_or("Untitled");

        let result = InlineQueryResultCachedVideo::new(Uuid::new_v4(), video_title, phantom_video_id.clone())
            .caption(HTML_DECORATION.code(HTML_DECORATION.quote(video_title).as_str()))
            .description("Click to send video")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Video downloading...").callback_data("video_downloading")
            ]]))
            .parse_mode(ParseMode::HTML)
            .into();

        results.push(result);
    }

    bot.send(
        AnswerInlineQuery::new(query_id, results)
            .is_personal(false)
            .cache_time(VIDEOS_IN_PLAYLIST_CACHE_TIME),
    )
    .await?;

    Ok(EventReturn::Finish)
}
