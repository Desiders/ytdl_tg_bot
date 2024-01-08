use crate::{
    cmd::ytdl,
    config::{PhantomAudioId, PhantomVideoId},
    errors::DownloadOrSendError,
    extractors::{BotConfigWrapper, YtDlpWrapper},
    handlers_utils::{chat_action, download, error, send},
    models::{AudioInFS, TgAudioInPlaylist, TgVideoInPlaylist, VideoInFS},
};

use std::sync::Arc;
use telers::{
    enums::ParseMode,
    errors::HandlerError,
    event::{telegram::HandlerResult, EventReturn},
    methods::{AnswerInlineQuery, EditMessageMedia, SendAudio, SendVideo},
    types::{
        ChosenInlineResult, InlineKeyboardButton, InlineKeyboardMarkup, InlineQuery, InlineQueryResult, InlineQueryResultCachedAudio,
        InlineQueryResultCachedVideo, InputFile, InputMediaVideo, Message,
    },
    utils::text::{html_code, html_quote},
    Bot, Context,
};
use tempfile::tempdir;
use tracing::{event, instrument, Level, Span};
use uuid::Uuid;

const SEND_VIDEO_TIMEOUT: f32 = 120.0; // 2 minutes
const SEND_AUDIO_TIMEOUT: f32 = 120.0; // 2 minutes
const SELECT_INLINE_QUERY_CACHE_TIME: i32 = 3600; // 60 minutes

#[instrument(skip_all, fields(message_id, chat_id, url))]
pub async fn video_download(
    bot: Arc<Bot>,
    context: Arc<Context>,
    message: Message,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    BotConfigWrapper(bot_config): BotConfigWrapper,
) -> HandlerResult {
    let url = context
        .get("video_url")
        .expect("Url should be in context because `text_contains_url` filter should do this")
        .downcast_ref::<Box<str>>()
        .expect("Url should be `Box<str>`")
        .clone()
        .into_string();
    let message_id = message.id();
    let chat_id = message.chat().id();

    Span::current()
        .record("chat_id", chat_id)
        .record("message_id", message_id)
        .record("url", url.as_str());

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            error::occured_in_message(
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

    let videos_len = videos.len();

    if videos_len == 0 {
        event!(Level::WARN, "Playlist doesn't have videos");

        error::occured_in_message(&bot, chat_id, message_id, "Playlist doesn't have videos.", None).await?;

        return Ok(EventReturn::Finish);
    }

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { chat_action::upload_video_action_in_loop(&bot, chat_id).await }
    });

    let mut handles = Vec::with_capacity(videos_len);

    for video in videos {
        let bot = bot.clone();
        let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();
        let receiver_video_chat_id = bot_config.receiver_video_chat_id;

        // This hack is needed because `ytdl` doesn't support downloading videos by ID from other sources, for example `coub.com `.
        // It also doesn't support uploading videos by direct URL, so we can only transmit the user's URL.
        // If URL represents playlist, we get an error because unacceptable use one URL one more time for different videos.
        // This should be fixed by direct download video without `ytdl`.
        let id_or_url = if videos_len == 1 { url.clone() } else { video.id.clone() };

        #[allow(clippy::cast_possible_truncation)]
        let (height, width, duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();

            HandlerError::new(err)
        })?;

        handles.push(tokio::spawn(async move {
            let VideoInFS { path, thumbnail_path } = download::video_to_temp_dir(
                video,
                id_or_url.as_str(),
                &temp_dir,
                max_files_size_in_bytes,
                yt_dlp_full_path.as_str(),
                false,
                true,
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendVideo::new(receiver_video_chat_id, InputFile::fs(path))
                    .disable_notification(true)
                    .width_option(width)
                    .height_option(height)
                    .duration_option(duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs))
                    .supports_streaming(true),
                Some(SEND_VIDEO_TIMEOUT),
            )
            .await?;

            Ok::<_, DownloadOrSendError>(message.video().unwrap().file_id.clone())
        }));
    }

    let mut videos_in_playlist = Vec::with_capacity(videos_len);
    let mut failed_downloads_count = 0;

    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(file_id)) => videos_in_playlist.push(TgVideoInPlaylist::new(file_id, index)),
            Ok(Err(err)) => {
                event!(Level::ERROR, %err, "Error while downloading video");

                failed_downloads_count += 1;
            }
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");

                failed_downloads_count += 1;
            }
        }
    }

    upload_action_task.abort();

    if failed_downloads_count > 0 {
        event!(Level::ERROR, "Failed downloads count is {failed_downloads_count}");

        error::download_videos_in_message(&bot, failed_downloads_count, chat_id, message_id, Some(ParseMode::HTML)).await?;
    }

    let input_media_list = {
        videos_in_playlist.sort_by(|a, b| a.index.cmp(&b.index));
        videos_in_playlist
            .into_iter()
            .map(|video| InputMediaVideo::new(InputFile::id(video.file_id.into_string())))
            .collect()
    };

    send::media_groups(&bot, chat_id, input_media_list, Some(message_id), Some(SEND_AUDIO_TIMEOUT)).await?;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(message_id, chat_id, url))]
pub async fn audio_download(
    bot: Arc<Bot>,
    context: Arc<Context>,
    message: Message,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    BotConfigWrapper(bot_config): BotConfigWrapper,
) -> HandlerResult {
    let url = context
        .get("video_url")
        .expect("Url should be in context because `text_contains_url` filter should do this")
        .downcast_ref::<Box<str>>()
        .expect("Url should be `Box<str>`")
        .clone()
        .into_string();
    let message_id = message.id();
    let chat_id = message.chat().id();

    Span::current()
        .record("url", url.as_str())
        .record("chat_id", chat_id)
        .record("message_id", message_id);

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting audio/playlist info");

            error::occured_in_message(
                &bot,
                chat_id,
                message_id,
                "Sorry, an error occurred while getting audio/playlist info. Try again later.",
                None,
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    let videos_len = videos.len();

    if videos_len == 0 {
        event!(Level::WARN, "Playlist doesn't have audios");

        error::occured_in_message(&bot, chat_id, message_id, "Playlist doesn't have audios.", None).await?;

        return Ok(EventReturn::Finish);
    }

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { chat_action::upload_voice_action_in_loop(&bot, chat_id).await }
    });

    let mut handles = Vec::with_capacity(videos_len);

    for video in videos {
        let bot = bot.clone();
        let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();
        let receiver_video_chat_id = bot_config.receiver_video_chat_id;

        // This hack is needed because `ytdl` doesn't support downloading videos by ID from other sources, for example `coub.com `.
        // It also doesn't support uploading videos by direct URL, so we can only transmit the passeds URL.
        // If URL represents playlist, we get an error because unacceptable use one URL one more time for different videos.
        // This should be fixed by direct download video without `ytdl`.
        let id_or_url = if videos_len == 1 { url.clone() } else { video.id.clone() };

        #[allow(clippy::cast_possible_truncation)]
        let duration = video.duration.map(|duration| duration as i64);

        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();

            HandlerError::new(err)
        })?;

        handles.push(tokio::spawn(async move {
            let title = video.title.clone();

            let AudioInFS { path, thumbnail_path } = download::audio_to_temp_dir(
                video,
                id_or_url.as_str(),
                &temp_dir,
                max_files_size_in_bytes,
                yt_dlp_full_path.as_str(),
                true,
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendAudio::new(receiver_video_chat_id, InputFile::fs(path))
                    .disable_notification(true)
                    .title_option(title)
                    .duration_option(duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                Some(SEND_AUDIO_TIMEOUT),
            )
            .await?;

            let file_id = if let Some(audio) = message.audio() {
                audio.file_id.as_ref()
            } else if let Some(voice) = message.voice() {
                voice.file_id.as_ref()
            } else {
                unreachable!("Message should have audio or voice")
            };

            Ok::<_, DownloadOrSendError>(file_id.to_owned())
        }));
    }

    let mut audios_in_playlist = Vec::with_capacity(videos_len);
    let mut failed_downloads_count = 0;

    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(file_id)) => audios_in_playlist.push(TgAudioInPlaylist::new(file_id, index)),
            Ok(Err(err)) => {
                event!(Level::ERROR, %err, "Error while downloading audio");

                failed_downloads_count += 1;
            }
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");

                failed_downloads_count += 1;
            }
        }
    }

    upload_action_task.abort();

    if failed_downloads_count > 0 {
        event!(Level::ERROR, "Failed downloads count is {failed_downloads_count}");

        error::download_audios_in_message(&bot, failed_downloads_count, chat_id, message_id, Some(ParseMode::HTML)).await?;
    }

    let input_media_list = {
        audios_in_playlist.sort_by(|a, b| a.index.cmp(&b.index));
        audios_in_playlist
            .into_iter()
            .map(|video| InputMediaVideo::new(InputFile::id(video.file_id.into_string())))
            .collect()
    };

    send::media_groups(&bot, chat_id, input_media_list, Some(message_id), Some(SEND_AUDIO_TIMEOUT)).await?;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(result_id, inline_message_id, video_id_or_url))]
pub async fn media_download_chosen_inline_result(
    bot: Arc<Bot>,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        query: url,
        ..
    }: ChosenInlineResult,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    BotConfigWrapper(bot_config): BotConfigWrapper,
) -> HandlerResult {
    Span::current().record("result_id", result_id.as_ref());
    Span::current().record("inline_message_id", inline_message_id.as_deref());
    Span::current().record("url", url.as_ref());

    // If `result_id` starts with `audio_` then it's audio, else it's video
    let download_video = result_id.starts_with("video_");

    let inline_message_id = inline_message_id.as_deref().unwrap();

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), false).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            error::occured_in_chosen_inline_result(
                &bot,
                "Sorry, an error occurred while getting video/playlist info. Try again later.",
                inline_message_id,
                None,
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    let Some(video) = videos.front().cloned() else {
        event!(Level::ERROR, "Video not found");

        error::occured_in_chosen_inline_result(&bot, "Sorry, video not found.", inline_message_id, None).await?;

        return Ok(EventReturn::Finish);
    };

    drop(videos);

    let handle: Result<(), HandlerError> = async {
        let temp_dir = tempdir().map_err(HandlerError::new)?;

        if download_video {
            #[allow(clippy::cast_possible_truncation)]
            let (height, width, duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

            let VideoInFS { path, thumbnail_path } = download::video_to_temp_dir(
                video,
                url.as_ref(),
                &temp_dir,
                yt_dlp_config.max_files_size_in_bytes,
                yt_dlp_config.full_path.as_str(),
                false,
                true,
            )
            .await
            .map_err(HandlerError::new)?;

            let message = send::with_retries(
                &bot,
                SendVideo::new(bot_config.receiver_video_chat_id, InputFile::fs(path))
                    .disable_notification(true)
                    .width_option(width)
                    .height_option(height)
                    .duration_option(duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs))
                    .supports_streaming(true),
                Some(SEND_VIDEO_TIMEOUT),
            )
            .await?;

            drop(temp_dir);

            send::with_retries(
                &bot,
                EditMessageMedia::new(InputMediaVideo::new(InputFile::id(message.video().unwrap().file_id.as_ref())))
                    .inline_message_id(inline_message_id)
                    .reply_markup(InlineKeyboardMarkup::new([[]])),
                Some(SEND_VIDEO_TIMEOUT),
            )
            .await?;
        } else {
            let title = video.title.clone();

            #[allow(clippy::cast_possible_truncation)]
            let duration = video.duration.map(|duration| duration as i64);

            let AudioInFS { path, thumbnail_path } = download::audio_to_temp_dir(
                video,
                url.as_ref(),
                &temp_dir,
                yt_dlp_config.max_files_size_in_bytes,
                yt_dlp_config.full_path.as_str(),
                true,
            )
            .await
            .map_err(HandlerError::new)?;

            let message = send::with_retries(
                &bot,
                SendAudio::new(bot_config.receiver_video_chat_id, InputFile::fs(path))
                    .disable_notification(true)
                    .title_option(title)
                    .duration_option(duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                Some(SEND_AUDIO_TIMEOUT),
            )
            .await?;

            drop(temp_dir);

            let file_id = if let Some(audio) = message.audio() {
                audio.file_id.as_ref()
            } else if let Some(voice) = message.voice() {
                voice.file_id.as_ref()
            } else {
                unreachable!("Message should have audio or voice")
            };

            send::with_retries(
                &bot,
                EditMessageMedia::new(InputMediaVideo::new(InputFile::id(file_id)))
                    .inline_message_id(inline_message_id)
                    .reply_markup(InlineKeyboardMarkup::new([[]])),
                Some(SEND_AUDIO_TIMEOUT),
            )
            .await?;
        }

        Ok(())
    }
    .await;

    if let Err(err) = handle {
        event!(Level::ERROR, %err, "Error while downloading media");

        error::occured_in_chosen_inline_result(
            &bot,
            "Sorry, an error occurred while downloading media. Try again later.",
            inline_message_id,
            None,
        )
        .await?;
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, url))]
pub async fn media_select_inline_query(
    bot: Arc<Bot>,
    InlineQuery {
        id: query_id, query: url, ..
    }: InlineQuery,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    PhantomVideoId(phantom_video_id): PhantomVideoId,
    PhantomAudioId(phantom_audio_id): PhantomAudioId,
) -> HandlerResult {
    Span::current().record("query_id", query_id.as_ref());
    Span::current().record("url", url.as_ref());

    event!(Level::DEBUG, "Got url");

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            error::occured_in_inline_query_occured(
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

        error::occured_in_inline_query_occured(&bot, query_id.as_ref(), "Playlist doesn't have videos.").await?;

        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos.len());

    for video in videos {
        let title = video.title.as_deref().unwrap_or("Untitled");
        let caption = html_code(html_quote(title));

        let result_id = Uuid::new_v4();

        results.push(
            InlineQueryResultCachedVideo::new(format!("video_{result_id}"), title, phantom_video_id.clone())
                .caption(caption.as_str())
                .description("Click to download video")
                .reply_markup(InlineKeyboardMarkup::new([[
                    InlineKeyboardButton::new("Video downloading...").callback_data("video_downloading")
                ]]))
                .parse_mode(ParseMode::HTML)
                .into(),
        );

        results.push(
            InlineQueryResultCachedAudio::new(format!("audio_{result_id}"), phantom_audio_id.clone())
                .caption(caption.as_str())
                .reply_markup(InlineKeyboardMarkup::new([[
                    InlineKeyboardButton::new("Audio downloading...").callback_data("audio_downloading")
                ]]))
                .parse_mode(ParseMode::HTML)
                .into(),
        );
    }

    bot.send(
        AnswerInlineQuery::new(query_id, results)
            .is_personal(false)
            .cache_time(SELECT_INLINE_QUERY_CACHE_TIME),
    )
    .await?;

    Ok(EventReturn::Finish)
}
