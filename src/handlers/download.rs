use crate::{
    cmd::get_media_or_playlist_info,
    config::{Bot as BotConfig, YtDlp},
    download::{self, AudioToTempDirErrorKind, VideoErrorKind},
    handlers_utils::{
        chat_action::{upload_video_action_in_loop, upload_voice_action_in_loop},
        error,
        range::Range,
        send::{self, with_retries},
        url::UrlWithParams,
    },
    interactors::{DownloadInfo, DownloadMedia, SendMedia},
    models::{AudioInFS, ShortInfo, Video, VideoInFS},
    services::yt_toolkit::{get_video_info, GetVideoInfoErrorKind},
    utils::format_error_report,
};

use either::Either;
use nix::libc;
use reqwest::Client;
use std::str::FromStr;
use telers::{
    enums::ParseMode,
    errors::{HandlerError, SessionErrorKind},
    event::{telegram::HandlerResult, EventReturn},
    methods::{AnswerInlineQuery, DeleteMessage, EditMessageMedia, SendAudio, SendMediaGroup, SendVideo},
    types::{
        ChosenInlineResult, InlineKeyboardButton, InlineKeyboardMarkup, InlineQuery, InlineQueryResult, InlineQueryResultArticle,
        InputFile, InputMediaAudio, InputMediaVideo, InputTextMessageContent, Message, ReplyParameters,
    },
    utils::text::{html_code, html_quote, html_text_link},
    Bot, Extension,
};
use tempfile::tempdir;
use tokio::{sync::mpsc, task::JoinError};
use tokio_util::task::AbortOnDropHandle;
use tracing::{event, field::debug, instrument, Level, Span};
use uuid::Uuid;

const GET_INFO_TIMEOUT: u64 = 120;
const DOWNLOAD_MEDIA_TIMEOUT: u64 = 180;
const SEND_VIDEO_TIMEOUT: f32 = 180.0;
const SEND_AUDIO_TIMEOUT: f32 = 180.0;
const GET_MEDIA_OR_PLAYLIST_INFO_INLINE_QUERY_TIMEOUT: u64 = 12;
const SELECT_INLINE_QUERY_CACHE_TIME: i64 = 86400; // 24 hours

#[allow(clippy::module_name_repetitions)]
#[derive(thiserror::Error, Debug)]
pub enum DownloadErrorKind {
    #[error(transparent)]
    Stream(#[from] VideoErrorKind),
    #[error(transparent)]
    Temp(#[from] AudioToTempDirErrorKind),
    #[error(transparent)]
    Session(#[from] SessionErrorKind),
    #[error(transparent)]
    Join(#[from] JoinError),
}

#[instrument(skip_all, fields(message_id, chat_id, url = url.as_str(), params))]
pub async fn video_download(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_config): Extension<YtDlp>,
    Extension(bot_config): Extension<BotConfig>,
) -> HandlerResult {
    let message_id = message.id();
    let chat_id = message.chat().id();

    Span::current()
        .record("chat_id", chat_id)
        .record("message_id", message_id)
        .record("params", debug(&params));

    event!(Level::DEBUG, "Got url");

    let range = match params.get("items") {
        Some(raw_value) => match Range::from_str(raw_value) {
            Ok(range) => range,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while parse range");

                error::occured_in_message(&bot, chat_id, message_id, &err.to_string(), None).await?;

                return Ok(EventReturn::Finish);
            }
        },
        None => Range::default(),
    };

    let _upload_action_task = AbortOnDropHandle::new(tokio::spawn({
        let bot = bot.clone();

        async move { upload_video_action_in_loop(&bot, chat_id).await }
    }));
    let temp_dir = tempdir().map_err(HandlerError::new)?;
    let (sender, receiver) = mpsc::unbounded_channel::<(_, Either<(VideoInFS, Video), _>)>();

    let DownloadInfo { is_playlist, count } =
        match DownloadMedia::new(url, range, true, temp_dir.path(), sender, &yt_dlp_config, &bot_config)
            .download()
            .await
        {
            Ok(info) => info,
            Err(err) => {
                error::occured_in_message(
                    &bot,
                    chat_id,
                    message_id,
                    &format!("Sorry, an error occurred while getting media info. {err}"),
                    None,
                )
                .await?;
                return Ok(EventReturn::Finish);
            }
        };

    if count == 0 {
        error::occured_in_message(&bot, chat_id, message_id, "Playlist empty", None).await?;
        return Ok(EventReturn::Finish);
    }

    match SendMedia::new(
        is_playlist,
        |(
            VideoInFS { path, thumbnail_path },
            Video {
                duration, width, height, ..
            },
        )| {
            let bot = bot.clone();
            let receiver_video_chat_id = bot_config.receiver_video_chat_id;
            async move {
                let message = match send::with_retries(
                    &bot,
                    SendVideo::new(receiver_video_chat_id, InputFile::fs(path))
                        .disable_notification(true)
                        .width_option(width)
                        .height_option(height)
                        .duration_option(duration.map(|duration| duration as i64))
                        .thumbnail_option(thumbnail_path.map(InputFile::fs))
                        .supports_streaming(true),
                    2,
                    Some(SEND_VIDEO_TIMEOUT),
                )
                .await
                {
                    Ok(message) => message,
                    Err(err) => return Err(err),
                };

                event!(Level::TRACE, "Video sended");

                tokio::spawn({
                    let message_id = message.id();

                    async move {
                        let _ = bot.send(DeleteMessage::new(receiver_video_chat_id, message_id)).await;
                    }
                });

                Ok(message.video().unwrap().file_id.to_string())
            }
        },
        {
            |media_ids| {
                let bot = bot.clone();
                async move {
                    let media_group = {
                        media_ids
                            .into_iter()
                            .map(|media_id| InputMediaVideo::new(InputFile::id(media_id)))
                            .collect::<Vec<_>>()
                    };

                    with_retries(
                        &bot,
                        SendMediaGroup::new(chat_id.clone(), media_group)
                            .disable_notification(true)
                            .reply_parameters(ReplyParameters::new(message_id).allow_sending_without_reply(true)),
                        3,
                        Some(SEND_VIDEO_TIMEOUT),
                    )
                    .await
                    .map(|_| ())
                }
            }
        },
        receiver,
    )
    .send()
    .await
    {
        Ok(_failed_sends) => {
            // TODO: Add message about failed sends
        }
        Err(err) => {
            error::occured_in_message(
                &bot,
                chat_id,
                message_id,
                &format!("Sorry, an error occurred while sending media group. {err}"),
                None,
            )
            .await?;
        }
    };

    drop(temp_dir);

    unsafe {
        libc::malloc_trim(0);
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(message_id, chat_id, url = url.as_str(), params))]
pub async fn video_download_quite(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_config): Extension<YtDlp>,
    Extension(bot_config): Extension<BotConfig>,
) -> HandlerResult {
    let message_id = message.id();
    let chat_id = message.chat().id();

    Span::current()
        .record("chat_id", chat_id)
        .record("message_id", message_id)
        .record("params", debug(&params));

    event!(Level::DEBUG, "Got url");

    let range = match params.get("items") {
        Some(raw_value) => match Range::from_str(raw_value) {
            Ok(range) => range,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while parse range");

                return Ok(EventReturn::Finish);
            }
        },
        None => Range::default(),
    };

    let temp_dir = tempdir().map_err(HandlerError::new)?;
    let (sender, receiver) = mpsc::unbounded_channel::<(_, Either<(VideoInFS, Video), _>)>();

    let Ok(DownloadInfo { is_playlist, count }) =
        DownloadMedia::new(url, range, true, temp_dir.path(), sender, &yt_dlp_config, &bot_config)
            .download()
            .await
    else {
        return Ok(EventReturn::Finish);
    };

    if count == 0 {
        return Ok(EventReturn::Finish);
    }

    match SendMedia::new(
        is_playlist,
        |(
            VideoInFS { path, thumbnail_path },
            Video {
                duration, width, height, ..
            },
        )| {
            let bot = bot.clone();
            let receiver_video_chat_id = bot_config.receiver_video_chat_id;
            async move {
                let message = match send::with_retries(
                    &bot,
                    SendVideo::new(receiver_video_chat_id, InputFile::fs(path))
                        .disable_notification(true)
                        .width_option(width)
                        .height_option(height)
                        .duration_option(duration.map(|duration| duration as i64))
                        .thumbnail_option(thumbnail_path.map(InputFile::fs))
                        .supports_streaming(true),
                    2,
                    Some(SEND_VIDEO_TIMEOUT),
                )
                .await
                {
                    Ok(message) => message,
                    Err(err) => return Err(err),
                };

                event!(Level::TRACE, "Video sended");

                tokio::spawn({
                    let message_id = message.id();

                    async move {
                        let _ = bot.send(DeleteMessage::new(receiver_video_chat_id, message_id)).await;
                    }
                });

                Ok(message.video().unwrap().file_id.to_string())
            }
        },
        {
            |media_ids| {
                let bot = bot.clone();
                async move {
                    let media_group = {
                        media_ids
                            .into_iter()
                            .map(|media_id| InputMediaVideo::new(InputFile::id(media_id)))
                            .collect::<Vec<_>>()
                    };

                    with_retries(
                        &bot,
                        SendMediaGroup::new(chat_id.clone(), media_group)
                            .disable_notification(true)
                            .reply_parameters(ReplyParameters::new(message_id).allow_sending_without_reply(true)),
                        3,
                        Some(SEND_VIDEO_TIMEOUT),
                    )
                    .await
                    .map(|_| ())
                }
            }
        },
        receiver,
    )
    .send()
    .await
    {
        Ok(_failed_sends) => {
            // TODO: Add message about failed sends
        }
        Err(_err) => {}
    };

    drop(temp_dir);

    unsafe {
        libc::malloc_trim(0);
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(message_id, chat_id, url = url.as_str(), params))]
pub async fn audio_download(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_config): Extension<YtDlp>,
    Extension(bot_config): Extension<BotConfig>,
) -> HandlerResult {
    let message_id = message.id();
    let chat_id = message.chat().id();

    Span::current()
        .record("chat_id", chat_id)
        .record("message_id", message_id)
        .record("params", debug(&params));

    event!(Level::DEBUG, "Got url");

    let range = match params.get("items") {
        Some(raw_value) => match Range::from_str(raw_value) {
            Ok(range) => range,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while parse range");

                error::occured_in_message(&bot, chat_id, message_id, &err.to_string(), None).await?;

                return Ok(EventReturn::Finish);
            }
        },
        None => Range::default(),
    };

    let _upload_action_task = AbortOnDropHandle::new(tokio::spawn({
        let bot = bot.clone();

        async move { upload_voice_action_in_loop(&bot, chat_id).await }
    }));
    let temp_dir = tempdir().map_err(HandlerError::new)?;
    let (sender, receiver) = mpsc::unbounded_channel::<(_, Either<(AudioInFS, Video), _>)>();

    let DownloadInfo { is_playlist, count } =
        match DownloadMedia::new(url, range, true, temp_dir.path(), sender, &yt_dlp_config, &bot_config)
            .download()
            .await
        {
            Ok(info) => info,
            Err(err) => {
                error::occured_in_message(
                    &bot,
                    chat_id,
                    message_id,
                    &format!("Sorry, an error occurred while getting media info. {err}"),
                    None,
                )
                .await?;
                return Ok(EventReturn::Finish);
            }
        };

    if count == 0 {
        error::occured_in_message(&bot, chat_id, message_id, "Playlist empty", None).await?;
        return Ok(EventReturn::Finish);
    }

    match SendMedia::new(
        is_playlist,
        |(AudioInFS { path, thumbnail_path }, Video { duration, title, .. })| {
            let bot = bot.clone();
            let receiver_video_chat_id = bot_config.receiver_video_chat_id;
            async move {
                let message = match send::with_retries(
                    &bot,
                    SendAudio::new(receiver_video_chat_id, InputFile::fs(path))
                        .disable_notification(true)
                        .title_option(title)
                        .duration_option(duration.map(|duration| duration as i64))
                        .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                    2,
                    Some(SEND_VIDEO_TIMEOUT),
                )
                .await
                {
                    Ok(message) => message,
                    Err(err) => return Err(err),
                };

                event!(Level::TRACE, "Audio sended");

                tokio::spawn({
                    let message_id = message.id();

                    async move {
                        let _ = bot.send(DeleteMessage::new(receiver_video_chat_id, message_id)).await;
                    }
                });

                let file_id = if let Some(audio) = message.audio() {
                    audio.file_id.clone().into_string()
                } else if let Some(voice) = message.voice() {
                    voice.file_id.clone().into_string()
                } else {
                    unreachable!("Message should have audio or voice")
                };
                Ok(file_id)
            }
        },
        {
            |media_ids| {
                let bot = bot.clone();
                async move {
                    let media_group = {
                        media_ids
                            .into_iter()
                            .map(|media_id| InputMediaAudio::new(InputFile::id(media_id)))
                            .collect::<Vec<_>>()
                    };

                    with_retries(
                        &bot,
                        SendMediaGroup::new(chat_id.clone(), media_group)
                            .disable_notification(true)
                            .reply_parameters(ReplyParameters::new(message_id).allow_sending_without_reply(true)),
                        3,
                        Some(SEND_VIDEO_TIMEOUT),
                    )
                    .await
                    .map(|_| ())
                }
            }
        },
        receiver,
    )
    .send()
    .await
    {
        Ok(_failed_sends) => {
            // TODO: Add message about failed sends
        }
        Err(err) => {
            error::occured_in_message(
                &bot,
                chat_id,
                message_id,
                &format!("Sorry, an error occurred while sending media group. {err}"),
                None,
            )
            .await?;
        }
    };

    drop(temp_dir);

    unsafe {
        libc::malloc_trim(0);
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(result_id, inline_message_id))]
pub async fn media_download_chosen_inline_result(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        query: url,
        ..
    }: ChosenInlineResult,
    Extension(yt_dlp_config): Extension<YtDlp>,
    Extension(bot_config): Extension<BotConfig>,
) -> HandlerResult {
    Span::current().record("result_id", result_id.as_ref());
    Span::current().record("inline_message_id", inline_message_id.as_deref());
    Span::current().record("url", url.as_ref());

    // If `result_id` starts with `audio_` then it's audio, else it's video
    let download_video = result_id.starts_with("video_");
    let inline_message_id = inline_message_id.as_deref().unwrap();

    event!(Level::DEBUG, "Got url");

    let videos = match get_media_or_playlist_info(&yt_dlp_config.full_path, &url, false, GET_INFO_TIMEOUT, &"1:1:1".parse().unwrap()).await
    {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Error while get info");

            error::occured_in_chosen_inline_result(
                &bot,
                "Sorry, an error occurred while getting media info. {err}",
                inline_message_id,
                None,
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    let Some(video) = videos.front().cloned() else {
        event!(Level::ERROR, "Video not found");

        error::occured_in_chosen_inline_result(&bot, "Sorry, video not found", inline_message_id, None).await?;

        return Ok(EventReturn::Finish);
    };

    drop(videos);

    event!(Level::DEBUG, "Got media info");

    let temp_dir = tempdir().map_err(HandlerError::new)?;

    let handle: Result<(), DownloadErrorKind> = async {
        if download_video {
            #[allow(clippy::cast_possible_truncation)]
            let (height, width, duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

            let VideoInFS { path, thumbnail_path } = download::video(
                &video,
                yt_dlp_config.max_file_size,
                yt_dlp_config.full_path,
                &bot_config.yt_toolkit_api_url,
                temp_dir.path(),
                DOWNLOAD_MEDIA_TIMEOUT,
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendVideo::new(bot_config.receiver_video_chat_id, InputFile::fs(path))
                    .disable_notification(true)
                    .width_option(width)
                    .height_option(height)
                    .duration_option(duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs))
                    .supports_streaming(true),
                2,
                Some(SEND_VIDEO_TIMEOUT),
            )
            .await?;

            drop(temp_dir);

            send::with_retries(
                &bot,
                EditMessageMedia::new(
                    InputMediaVideo::new(InputFile::id(message.video().unwrap().file_id.as_ref()))
                        .caption(html_text_link("Link", url))
                        .parse_mode(ParseMode::HTML),
                )
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[]])),
                2,
                Some(SEND_VIDEO_TIMEOUT),
            )
            .await?;

            tokio::spawn({
                let message_id = message.id();
                let bot = bot.clone();

                async move {
                    let _ = bot.send(DeleteMessage::new(bot_config.receiver_video_chat_id, message_id)).await;
                }
            });
        } else {
            let title = video.title.clone();

            #[allow(clippy::cast_possible_truncation)]
            let duration = video.duration.map(|duration| duration as i64);

            let AudioInFS { path, thumbnail_path } = download::audio_to_temp_dir(
                &video,
                &url,
                yt_dlp_config.max_file_size,
                &yt_dlp_config.full_path,
                &bot_config.yt_toolkit_api_url,
                temp_dir.path(),
                DOWNLOAD_MEDIA_TIMEOUT,
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendAudio::new(bot_config.receiver_video_chat_id, InputFile::fs(path))
                    .disable_notification(true)
                    .title_option(title)
                    .duration_option(duration)
                    .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                2,
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

            send::with_retries(
                &bot,
                EditMessageMedia::new(
                    InputMediaVideo::new(InputFile::id(file_id))
                        .caption(html_text_link("Link", url))
                        .parse_mode(ParseMode::HTML),
                )
                .inline_message_id(inline_message_id),
                2,
                Some(SEND_AUDIO_TIMEOUT),
            )
            .await?;

            tokio::spawn({
                let message_id = message.id();
                let bot = bot.clone();

                async move {
                    let _ = bot.send(DeleteMessage::new(bot_config.receiver_video_chat_id, message_id)).await;
                }
            });
        }

        Ok(())
    }
    .await;

    if let Err(err) = handle {
        event!(Level::ERROR, err = format_error_report(&err), "Error while download media");

        error::occured_in_chosen_inline_result(&bot, "Sorry, an error occurred while downloading media", inline_message_id, None).await?;
    }

    unsafe {
        libc::malloc_trim(0);
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, url))]
pub async fn media_select_inline_query(
    bot: Bot,
    InlineQuery {
        id: query_id, query: url, ..
    }: InlineQuery,
    Extension(yt_dlp_config): Extension<YtDlp>,
    Extension(bot_config): Extension<BotConfig>,
) -> HandlerResult {
    Span::current().record("query_id", query_id.as_ref());
    Span::current().record("url", url.as_ref());

    event!(Level::DEBUG, "Got url");

    let videos_titles: Vec<ShortInfo> = match get_video_info(Client::new(), &bot_config.yt_toolkit_api_url, &url).await {
        Ok(videos_titles) => videos_titles.into_iter().map(Into::into).collect(),
        Err(err) => {
            match err {
                GetVideoInfoErrorKind::GetVideoId(err) => event!(Level::ERROR, %err, "Unsupported URL for YT Toolkit"),
                _ => event!(Level::ERROR, err = format_error_report(&err), "Getting media info YT Toolkit error"),
            };

            match get_media_or_playlist_info(
                &yt_dlp_config.full_path,
                url,
                true,
                GET_MEDIA_OR_PLAYLIST_INFO_INLINE_QUERY_TIMEOUT,
                &"1:1:1".parse().unwrap(),
            )
            .await
            {
                Ok(videos) => videos.map(Into::into).collect(),
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Getting media info error");

                    error::occured_in_chosen_inline_result(
                        &bot,
                        "Sorry, an error occurred while getting media info",
                        query_id.as_ref(),
                        None,
                    )
                    .await?;

                    return Ok(EventReturn::Finish);
                }
            }
        }
    };

    let videos_len = videos_titles.len();

    if videos_len == 0 {
        event!(Level::WARN, "Playlist empty");

        error::occured_in_inline_query_occured(&bot, query_id.as_ref(), "Playlist empty").await?;

        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, videos_len, "Got media info");

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos_len);

    for ShortInfo { title, thumbnails } in videos_titles {
        let title = title.as_deref().unwrap_or("Untitled");
        let title_html = html_code(html_quote(title));
        let thumbnail_url = thumbnails.first().map(|thumbnail| thumbnail.url.as_deref()).flatten();
        let result_id = Uuid::new_v4();

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{result_id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .title(title)
            .thumbnail_url_option(thumbnail_url)
            .description("Click to download video")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading...").callback_data("video_download")
            ]]))
            .into(),
        );
        results.push(
            InlineQueryResultArticle::new(
                format!("audio_{result_id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .title(title)
            .thumbnail_url_option(thumbnail_url)
            .description("Click to download audio")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading...").callback_data("audio_download")
            ]]))
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
