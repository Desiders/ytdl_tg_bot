use crate::{
    cmd::ytdl,
    config::{PhantomAudioId, PhantomVideoId},
    errors::DownloadOrSendError,
    extractors::{BotConfigWrapper, YtDlpWrapper},
    handlers_utils::{chat_action, download, error, media_group},
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
    utils::text_decorations::{TextDecoration, HTML_DECORATION},
    Bot, Context,
};
use tempfile::tempdir;
use tracing::{event, field, instrument, Level, Span};
use uuid::Uuid;

const SEND_VIDEO_TIMEOUT: f32 = 120.0; // 2 minutes
const SEND_AUDIO_TIMEOUT: f32 = 120.0; // 2 minute
const SELECT_INLINE_QUERY_CACHE_TIME: i32 = 1; // 10 minutes

#[instrument(skip_all, fields(message_id, chat_id = chat.id, url))]
pub async fn video_download(
    bot: Arc<Bot>,
    context: Arc<Context>,
    Message { message_id, chat, .. }: Message,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    BotConfigWrapper(bot_config): BotConfigWrapper,
) -> HandlerResult {
    let url = context
        .get("video_url")
        .expect("Url should be in context because `text_contains_url` filter should do this")
        .downcast_ref::<Box<str>>()
        .expect("Url should be `Box<str>`")
        .clone();
    let chat_id = chat.id;

    event!(Level::DEBUG, "Got url");

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { chat_action::upload_video_action_in_loop(&bot, chat_id).await }
    });

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            upload_action_task.abort();

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

        upload_action_task.abort();

        error::occured_in_message(&bot, chat_id, message_id, "Playlist doesn't have videos.", None).await?;

        return Ok(EventReturn::Finish);
    }

    let mut handles = Vec::with_capacity(videos_len);

    for (video_index, video) in videos.enumerate() {
        let bot = bot.clone();
        let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();
        let receiver_video_chat_id = bot_config.receiver_video_chat_id;

        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();

            HandlerError::new(err)
        })?;

        handles.push(tokio::spawn(async move {
            #[allow(clippy::cast_possible_truncation)]
            let (height, width, video_duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

            let VideoInFS {
                path: file_path,
                thumbnail_path,
            } = download::video_to_temp_dir(video, &temp_dir, max_files_size_in_bytes, yt_dlp_full_path.as_str(), false, true).await?;

            let Message { video, .. } = bot
                .send_with_timeout(
                    SendVideo::new(receiver_video_chat_id, InputFile::fs(file_path))
                        .disable_notification(true)
                        .width_option(width)
                        .height_option(height)
                        .duration_option(video_duration)
                        .thumbnail_option(thumbnail_path.map(InputFile::fs))
                        .supports_streaming(true),
                    SEND_VIDEO_TIMEOUT,
                )
                .await?;

            Ok::<_, DownloadOrSendError>(TgVideoInPlaylist::new(video.unwrap().file_id, video_index))
        }));
    }

    let mut videos_in_playlist = Vec::with_capacity(videos_len);

    for handle in handles {
        match handle.await {
            Ok(Ok(video_in_playlist)) => videos_in_playlist.push(video_in_playlist),
            Ok(Err(err)) => {
                event!(Level::ERROR, %err, "Error while downloading video");
            }
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");
            }
        }
    }

    upload_action_task.abort();

    let input_media_list = {
        videos_in_playlist.sort_by(|a, b| a.index_in_playlist.cmp(&b.index_in_playlist));
        videos_in_playlist
            .into_iter()
            .map(|video| InputMediaVideo::new(InputFile::id(video.file_id.into_string())))
            .collect()
    };

    media_group::send_from_input_media_list(&bot, chat_id, input_media_list, Some(message_id)).await?;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(message_id, chat_id = chat.id, url))]
pub async fn audio_download(
    bot: Arc<Bot>,
    context: Arc<Context>,
    Message { message_id, chat, .. }: Message,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
    BotConfigWrapper(bot_config): BotConfigWrapper,
) -> HandlerResult {
    let url = context
        .get("video_url")
        .expect("Url should be in context because `text_contains_url` filter should do this")
        .downcast_ref::<Box<str>>()
        .expect("Url should be `Box<str>`")
        .clone();
    let chat_id = chat.id;

    event!(Level::DEBUG, "Got url");

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { chat_action::upload_video_action_in_loop(&bot, chat_id).await }
    });

    let videos = match ytdl::get_video_or_playlist_info(&yt_dlp_config.full_path, url.as_ref(), true).await {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, %err, "Error while getting video/playlist info");

            upload_action_task.abort();

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

        upload_action_task.abort();

        error::occured_in_message(&bot, chat_id, message_id, "Playlist doesn't have videos.", None).await?;

        return Ok(EventReturn::Finish);
    }

    let mut handles = Vec::with_capacity(videos_len);

    for (video_index, video) in videos.enumerate() {
        let bot = bot.clone();
        let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
        let yt_dlp_full_path = yt_dlp_config.as_ref().full_path.clone();
        let receiver_video_chat_id = bot_config.receiver_video_chat_id;

        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();

            HandlerError::new(err)
        })?;

        handles.push(tokio::spawn(async move {
            #[allow(clippy::cast_possible_truncation)]
            let video_duration = video.duration.map(|duration| duration as i64);

            let AudioInFS {
                path: file_path,
                thumbnail_path,
            } = download::audio_to_temp_dir(video, &temp_dir, max_files_size_in_bytes, yt_dlp_full_path.as_str(), true).await?;

            let Message { audio, .. } = bot
                .send_with_timeout(
                    SendAudio::new(receiver_video_chat_id, InputFile::fs(file_path))
                        .disable_notification(true)
                        .duration_option(video_duration)
                        .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                    SEND_AUDIO_TIMEOUT,
                )
                .await?;

            Ok::<_, DownloadOrSendError>(TgAudioInPlaylist::new(audio.unwrap().file_id, video_index))
        }));
    }

    let mut audios_in_playlist = Vec::with_capacity(videos_len);

    for handle in handles {
        match handle.await {
            Ok(Ok(audio_in_playlist)) => audios_in_playlist.push(audio_in_playlist),
            Ok(Err(err)) => {
                event!(Level::ERROR, %err, "Error while downloading video");
            }
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");
            }
        }
    }

    upload_action_task.abort();

    let input_media_list = {
        audios_in_playlist.sort_by(|a, b| a.index_in_playlist.cmp(&b.index_in_playlist));
        audios_in_playlist
            .into_iter()
            .map(|video| InputMediaVideo::new(InputFile::id(video.file_id.into_string())))
            .collect()
    };

    media_group::send_from_input_media_list(&bot, chat_id, input_media_list, Some(message_id)).await?;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(result_id = field::Empty, inline_message_id = field::Empty, url = field::Empty))]
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

    let temp_dir = tempdir().map_err(HandlerError::new)?;

    if download_video {
        #[allow(clippy::cast_possible_truncation)]
        let (height, width, video_duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

        let VideoInFS {
            path: file_path,
            thumbnail_path,
        } = download::video_to_temp_dir(
            video,
            &temp_dir,
            yt_dlp_config.max_files_size_in_bytes,
            yt_dlp_config.full_path.as_str(),
            false,
            true,
        )
        .await
        .map_err(HandlerError::new)?;

        let Message { video, .. } = bot
            .send_with_timeout(
                SendVideo::new(bot_config.receiver_video_chat_id, InputFile::fs(file_path))
                    .disable_notification(true)
                    .width_option(width)
                    .height_option(height)
                    .duration_option(video_duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs))
                    .supports_streaming(true),
                SEND_VIDEO_TIMEOUT,
            )
            .await?;

        drop(temp_dir);

        bot.send_with_timeout(
            EditMessageMedia::new(InputMediaVideo::new(InputFile::id(video.unwrap().file_id.as_ref())))
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[]])),
            SEND_VIDEO_TIMEOUT,
        )
        .await?;
    } else {
        #[allow(clippy::cast_possible_truncation)]
        let video_duration = video.duration.map(|duration| duration as i64);

        let AudioInFS {
            path: file_path,
            thumbnail_path,
        } = download::audio_to_temp_dir(
            video,
            &temp_dir,
            yt_dlp_config.max_files_size_in_bytes,
            yt_dlp_config.full_path.as_str(),
            true,
        )
        .await
        .map_err(HandlerError::new)?;

        let Message { audio, .. } = bot
            .send_with_timeout(
                SendAudio::new(bot_config.receiver_video_chat_id, InputFile::fs(file_path))
                    .disable_notification(true)
                    .duration_option(video_duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                SEND_AUDIO_TIMEOUT,
            )
            .await?;

        drop(temp_dir);

        bot.send_with_timeout(
            EditMessageMedia::new(InputMediaVideo::new(InputFile::id(audio.unwrap().file_id.as_ref())))
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[]])),
            SEND_AUDIO_TIMEOUT,
        )
        .await?;
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id = field::Empty, url = field::Empty))]
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
        let video_title = video.title.as_deref().unwrap_or("Untitled");

        let result_id = Uuid::new_v4();
        let video_result_id = format!("video_{result_id}");
        let audio_result_id = format!("audio_{result_id}");

        let video_result = InlineQueryResultCachedVideo::new(video_result_id, video_title, phantom_video_id.clone())
            .caption(HTML_DECORATION.code(HTML_DECORATION.quote(video_title).as_str()))
            .description("Click to download video")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Video downloading...").callback_data("video_downloading")
            ]]))
            .parse_mode(ParseMode::HTML)
            .into();

        results.push(video_result);

        let audio_result = InlineQueryResultCachedAudio::new(audio_result_id, phantom_audio_id.clone())
            .caption(HTML_DECORATION.code(HTML_DECORATION.quote(video_title).as_str()))
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Audio downloading...").callback_data("audio_downloading")
            ]]))
            .parse_mode(ParseMode::HTML)
            .into();

        results.push(audio_result);
    }

    bot.send(
        AnswerInlineQuery::new(query_id, results)
            .is_personal(false)
            .cache_time(SELECT_INLINE_QUERY_CACHE_TIME),
    )
    .await?;

    Ok(EventReturn::Finish)
}
