use crate::{
    config::{ChatConfig, YtDlpConfig, YtToolkitConfig},
    download::{self, StreamErrorKind, ToTempDirErrorKind},
    handlers_utils::{
        chat_action::{upload_video_action_in_loop, upload_voice_action_in_loop},
        error,
        preferred_languages::PreferredLanguages,
        range::Range,
        send,
        url::UrlWithParams,
    },
    models::{AudioInFS, Cookies, ShortInfo, TgAudioInPlaylist, TgVideoInPlaylist, VideoInFS},
    services::{
        get_media_or_playlist_info,
        yt_toolkit::{get_video_info, search_video, GetVideoInfoErrorKind},
    },
    utils::format_error_report,
};

use reqwest::Client;
use std::str::FromStr;
use telers::{
    enums::ParseMode,
    errors::{HandlerError, SessionErrorKind},
    event::{telegram::HandlerResult, EventReturn},
    methods::{AnswerInlineQuery, DeleteMessage, EditMessageMedia, SendAudio, SendVideo},
    types::{
        ChosenInlineResult, InlineKeyboardButton, InlineKeyboardMarkup, InlineQuery, InlineQueryResult, InlineQueryResultArticle,
        InputFile, InputMediaVideo, InputTextMessageContent, Message,
    },
    utils::text::{html_code, html_quote, html_text_link},
    Bot, Extension,
};
use tempfile::tempdir;
use tokio::task::{spawn_blocking, JoinError, JoinHandle};
use tracing::{event, field::debug, instrument, Level, Span};
use url::{Host, Url};
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
    Stream(#[from] StreamErrorKind),
    #[error(transparent)]
    Temp(#[from] ToTempDirErrorKind),
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
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(cookies): Extension<Cookies>,
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
    let preferred_languages = match params.get("lang") {
        Some(raw_value) => match PreferredLanguages::from_str(raw_value) {
            Ok(languages) => languages,
            Err(err) => {
                event!(
                    Level::ERROR,
                    err = format_error_report(&err),
                    "Error while parse preferred languages"
                );

                error::occured_in_message(&bot, chat_id, message_id, &err.to_string(), None).await?;
                return Ok(EventReturn::Finish);
            }
        },
        None => PreferredLanguages::default(),
    };

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { upload_video_action_in_loop(&bot, chat_id).await }
    });

    let cookie = cookies.get_path_by_optional_host(url.host().as_ref());

    let videos = match spawn_blocking({
        let path = yt_dlp_cfg.executable_path.clone();
        let url = url.clone();
        let cookie = cookie.cloned();

        event!(Level::DEBUG, host = ?url.host(), "Getting media info with yt-dlp");

        move || get_media_or_playlist_info(path, url, true, GET_INFO_TIMEOUT, &range, cookie.as_ref())
    })
    .await
    .map_err(|err| {
        upload_action_task.abort();
        event!(Level::ERROR, err = format_error_report(&err), "Error while join");
        HandlerError::new(err)
    })? {
        Ok(videos) => videos,
        Err(err) => {
            upload_action_task.abort();
            event!(Level::ERROR, err = format_error_report(&err), "Error while get info");

            error::occured_in_message(&bot, chat_id, message_id, "Sorry, an error occurred while getting media info", None).await?;
            return Ok(EventReturn::Finish);
        }
    };

    let videos_len = videos.len();

    if videos_len == 0 {
        upload_action_task.abort();
        event!(Level::WARN, "Playlist empty");

        error::occured_in_message(&bot, chat_id, message_id, "Playlist empty", None).await?;
        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, videos_len, "Got media info");

    let mut failed_downloads_count = 0;
    let mut handles: Vec<JoinHandle<Result<_, DownloadErrorKind>>> = Vec::with_capacity(videos_len);

    for video in videos {
        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();

            HandlerError::new(err)
        })?;

        #[allow(clippy::cast_possible_truncation)]
        let (height, width, duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

        let VideoInFS { path, thumbnail_path } = match download::video(
            video,
            yt_dlp_cfg.max_file_size,
            &yt_dlp_cfg.executable_path,
            &yt_toolkit_cfg.url,
            temp_dir.path(),
            url.host().is_some_and(|host| match host {
                Host::Domain(domain) => domain.contains("youtube") || domain == "youtu.be",
                _ => false,
            }),
            DOWNLOAD_MEDIA_TIMEOUT,
            cookie,
            &preferred_languages.languages.iter().map(AsRef::as_ref).collect::<Box<[_]>>(),
        )
        .await
        {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while download");
                failed_downloads_count += 1;
                continue;
            }
        };

        event!(Level::TRACE, "Send video");

        handles.push({
            let bot = bot.clone();
            let chat_id = chat_cfg.receiver_chat_id;

            tokio::spawn(async move {
                let message = send::with_retries(
                    &bot,
                    SendVideo::new(chat_id, InputFile::fs(path))
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

                // Don't delete this line, we need it to avoid drop
                drop(temp_dir);

                event!(Level::TRACE, "Video sended");

                tokio::spawn({
                    let message_id = message.id();

                    async move {
                        let _ = bot.send(DeleteMessage::new(chat_id, message_id)).await;
                    }
                });

                Ok(message.video().unwrap().file_id.clone())
            })
        });
    }

    let mut videos_in_playlist = Vec::with_capacity(videos_len);

    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(file_id)) => videos_in_playlist.push(TgVideoInPlaylist::new(file_id, index)),
            Ok(Err(err)) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while download");

                failed_downloads_count += 1;
            }
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while join");

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

#[instrument(skip_all, fields(message_id, chat_id, url = url.as_str(), params))]
pub async fn video_download_quite(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(cookies): Extension<Cookies>,
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
    let preferred_languages = match params.get("lang") {
        Some(raw_value) => match PreferredLanguages::from_str(raw_value) {
            Ok(languages) => languages,
            Err(err) => {
                event!(
                    Level::ERROR,
                    err = format_error_report(&err),
                    "Error while parse preferred languages"
                );

                error::occured_in_message(&bot, chat_id, message_id, &err.to_string(), None).await?;
                return Ok(EventReturn::Finish);
            }
        },
        None => PreferredLanguages::default(),
    };

    let cookie = cookies.get_path_by_optional_host(url.host().as_ref());

    let videos = match spawn_blocking({
        let path = yt_dlp_cfg.executable_path.clone();
        let url = url.clone();
        let cookie = cookie.cloned();

        move || get_media_or_playlist_info(path, url, true, GET_INFO_TIMEOUT, &range, cookie.as_ref())
    })
    .await
    .map_err(|err| {
        event!(Level::ERROR, err = format_error_report(&err), "Error while join");

        HandlerError::new(err)
    })? {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Error while get info");

            return Ok(EventReturn::Finish);
        }
    };

    let videos_len = videos.len();

    if videos_len == 0 {
        event!(Level::WARN, "Playlist empty");

        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, videos_len, "Got media info");

    let mut failed_downloads_count = 0;
    let mut handles: Vec<JoinHandle<Result<_, DownloadErrorKind>>> = Vec::with_capacity(videos_len);

    for video in videos {
        let temp_dir = tempdir().map_err(HandlerError::new)?;

        #[allow(clippy::cast_possible_truncation)]
        let (height, width, duration) = (video.height, video.width, video.duration.map(|duration| duration as i64));

        let VideoInFS { path, thumbnail_path } = match download::video(
            video,
            yt_dlp_cfg.max_file_size,
            &yt_dlp_cfg.executable_path,
            &yt_toolkit_cfg.url,
            temp_dir.path(),
            url.host().is_some_and(|host| match host {
                Host::Domain(domain) => domain.contains("youtube") || domain == "youtu.be",
                _ => false,
            }),
            DOWNLOAD_MEDIA_TIMEOUT,
            cookie,
            &preferred_languages.languages.iter().map(AsRef::as_ref).collect::<Box<[_]>>(),
        )
        .await
        {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while download");
                failed_downloads_count += 1;
                continue;
            }
        };

        event!(Level::TRACE, "Send video");

        handles.push({
            let bot = bot.clone();
            let chat_id = chat_cfg.receiver_chat_id;

            tokio::spawn(async move {
                let message = send::with_retries(
                    &bot,
                    SendVideo::new(chat_id, InputFile::fs(path))
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

                // Don't delete this line, we need it to avoid drop
                drop(temp_dir);

                event!(Level::TRACE, "Video sended");

                tokio::spawn({
                    let message_id = message.id();

                    async move {
                        let _ = bot.send(DeleteMessage::new(chat_id, message_id)).await;
                    }
                });

                Ok(message.video().unwrap().file_id.clone())
            })
        });
    }

    let mut videos_in_playlist = Vec::with_capacity(videos_len);

    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(file_id)) => videos_in_playlist.push(TgVideoInPlaylist::new(file_id, index)),
            Ok(Err(err)) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while download");

                failed_downloads_count += 1;
            }
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while join");

                failed_downloads_count += 1;
            }
        }
    }

    if failed_downloads_count > 0 {
        event!(Level::ERROR, "Failed downloads count is {failed_downloads_count}");
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

#[instrument(skip_all, fields(message_id, chat_id, url = url.as_str(), params))]
pub async fn audio_download(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(cookies): Extension<Cookies>,
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
    let preferred_languages = match params.get("lang") {
        Some(raw_value) => match PreferredLanguages::from_str(raw_value) {
            Ok(languages) => languages,
            Err(err) => {
                event!(
                    Level::ERROR,
                    err = format_error_report(&err),
                    "Error while parse preferred languages"
                );

                error::occured_in_message(&bot, chat_id, message_id, &err.to_string(), None).await?;
                return Ok(EventReturn::Finish);
            }
        },
        None => PreferredLanguages::default(),
    };

    let upload_action_task = tokio::spawn({
        let bot = bot.clone();

        async move { upload_voice_action_in_loop(&bot, chat_id).await }
    });

    let cookie = cookies.get_path_by_optional_host(url.host().as_ref());

    let videos = match spawn_blocking({
        let path = yt_dlp_cfg.executable_path.clone();
        let url = url.clone();
        let cookie = cookie.cloned();

        move || get_media_or_playlist_info(path, url, true, GET_INFO_TIMEOUT, &range, cookie.as_ref())
    })
    .await
    .map_err(|err| {
        upload_action_task.abort();
        event!(Level::ERROR, err = format_error_report(&err), "Error while join");
        HandlerError::new(err)
    })? {
        Ok(videos) => videos,
        Err(err) => {
            upload_action_task.abort();
            event!(Level::ERROR, err = format_error_report(&err), "Error while get info");

            error::occured_in_message(&bot, chat_id, message_id, "Sorry, an error occurred while getting media info", None).await?;
            return Ok(EventReturn::Finish);
        }
    };

    let videos_len = videos.len();

    if videos_len == 0 {
        upload_action_task.abort();
        event!(Level::WARN, "Playlist empty");

        error::occured_in_message(&bot, chat_id, message_id, "Playlist empty", None).await?;
        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, videos_len, "Got media info");

    let mut failed_downloads_count = 0;
    let mut handles: Vec<JoinHandle<Result<Box<str>, DownloadErrorKind>>> = Vec::with_capacity(videos_len);

    for video in videos {
        let temp_dir = tempdir().map_err(|err| {
            upload_action_task.abort();
            HandlerError::new(err)
        })?;

        // This hack is needed because `ytdl` doesn't support downloading videos by ID from other sources, for example `coub.com `.
        // It also doesn't support uploading videos by direct URL, so we can only transmit the passeds URL.
        // If URL represents playlist, we get an error because unacceptable use one URL one more time for different videos.
        // This should be fixed by direct download video without `ytdl`.
        let id_or_url = if videos_len == 1 {
            url.as_str().to_owned()
        } else {
            video.id.clone()
        };
        let title = video.title.clone();

        #[allow(clippy::cast_possible_truncation)]
        let duration = video.duration.map(|duration| duration as i64);

        let AudioInFS { path, thumbnail_path } = match download::audio_to_temp_dir(
            video,
            id_or_url,
            yt_dlp_cfg.max_file_size,
            &yt_dlp_cfg.executable_path,
            &yt_toolkit_cfg.url,
            temp_dir.path(),
            url.host().is_some_and(|host| match host {
                Host::Domain(domain) => domain.contains("youtube") || domain == "youtu.be",
                _ => false,
            }),
            DOWNLOAD_MEDIA_TIMEOUT,
            cookie,
            &preferred_languages.languages.iter().map(AsRef::as_ref).collect::<Box<[_]>>(),
        )
        .await
        {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while download");
                failed_downloads_count += 1;
                continue;
            }
        };

        handles.push({
            let bot = bot.clone();
            let chat_id = chat_cfg.receiver_chat_id;

            tokio::spawn(async move {
                let message = send::with_retries(
                    &bot,
                    SendAudio::new(chat_id, InputFile::fs(path))
                        .disable_notification(true)
                        .title_option(title)
                        .duration_option(duration)
                        .thumbnail_option(thumbnail_path.map(InputFile::fs)),
                    2,
                    Some(SEND_AUDIO_TIMEOUT),
                )
                .await?;

                // Don't delete this line, we need it to avoid drop
                drop(temp_dir);

                tokio::spawn({
                    let message_id = message.id();

                    async move {
                        let _ = bot.send(DeleteMessage::new(chat_id, message_id)).await;
                    }
                });

                let file_id = if let Some(audio) = message.audio() {
                    audio.file_id.as_ref()
                } else if let Some(voice) = message.voice() {
                    voice.file_id.as_ref()
                } else {
                    unreachable!("Message should have audio or voice")
                };

                Ok(file_id.to_owned().into_boxed_str())
            })
        });
    }

    let mut audios_in_playlist = Vec::with_capacity(videos_len);

    for (index, handle) in handles.into_iter().enumerate() {
        match handle.await {
            Ok(Ok(file_id)) => audios_in_playlist.push(TgAudioInPlaylist::new(file_id, index)),
            Ok(Err(err)) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while download audio");

                failed_downloads_count += 1;
            }
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Error while join");

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

#[instrument(skip_all, fields(result_id, inline_message_id))]
pub async fn media_download_chosen_inline_result(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        query: url,
        ..
    }: ChosenInlineResult,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(cookies): Extension<Cookies>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();

    let Ok(url) = Url::parse(&url) else {
        error::occured_in_chosen_inline_result(&bot, "Sorry, video not found", inline_message_id, None).await?;
        return Ok(EventReturn::Finish);
    };

    Span::current().record("result_id", result_id.as_ref());
    Span::current().record("inline_message_id", inline_message_id);
    Span::current().record("url", url.as_str());

    // If `result_id` starts with `audio_` then it's audio, else it's video
    let download_video = result_id.starts_with("video_");
    let cookie = cookies.get_path_by_optional_host(url.host().as_ref());

    event!(Level::DEBUG, "Got url");

    let videos = match spawn_blocking({
        let path = yt_dlp_cfg.executable_path.clone();
        let url = url.clone();
        let cookie = cookie.cloned();

        move || get_media_or_playlist_info(path, url, false, GET_INFO_TIMEOUT, &"1:1:1".parse().unwrap(), cookie.as_ref())
    })
    .await
    .map_err(HandlerError::new)?
    {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Error while get info");

            error::occured_in_chosen_inline_result(&bot, "Sorry, an error occurred while getting media info", inline_message_id, None)
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
                video,
                yt_dlp_cfg.max_file_size,
                yt_dlp_cfg.executable_path,
                &yt_toolkit_cfg.url,
                temp_dir.path(),
                url.host().is_some_and(|host| match host {
                    Host::Domain(domain) => domain.contains("youtube") || domain == "youtu.be",
                    _ => false,
                }),
                DOWNLOAD_MEDIA_TIMEOUT,
                cookie,
                &PreferredLanguages::default()
                    .languages
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Box<[_]>>(),
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendVideo::new(chat_cfg.receiver_chat_id, InputFile::fs(path))
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
                    let _ = bot.send(DeleteMessage::new(chat_cfg.receiver_chat_id, message_id)).await;
                }
            });
        } else {
            let title = video.title.clone();

            #[allow(clippy::cast_possible_truncation)]
            let duration = video.duration.map(|duration| duration as i64);

            let AudioInFS { path, thumbnail_path } = download::audio_to_temp_dir(
                video,
                &url,
                yt_dlp_cfg.max_file_size,
                &yt_dlp_cfg.executable_path,
                &yt_toolkit_cfg.url,
                temp_dir.path(),
                url.domain()
                    .is_some_and(|domain| domain.contains("youtube") || domain == "youtu.be"),
                DOWNLOAD_MEDIA_TIMEOUT,
                cookie,
                &PreferredLanguages::default()
                    .languages
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Box<[_]>>(),
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendAudio::new(chat_cfg.receiver_chat_id, InputFile::fs(path))
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
                    let _ = bot.send(DeleteMessage::new(chat_cfg.receiver_chat_id, message_id)).await;
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

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(result_id, inline_message_id))]
pub async fn media_download_search_chosen_inline_result(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        ..
    }: ChosenInlineResult,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(cookies): Extension<Cookies>,
) -> HandlerResult {
    Span::current().record("result_id", result_id.as_ref());
    Span::current().record("inline_message_id", inline_message_id.as_deref());

    let (result_prefix, video_id) = result_id.split_once('_').unwrap();

    let download_video = result_prefix.starts_with("video");
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let cookie = cookies.get_path_by_host(&Host::Domain("youtube.com"));

    event!(Level::DEBUG, "Got url");

    let videos = match spawn_blocking({
        let path = yt_dlp_cfg.executable_path.clone();
        let video_id = video_id.to_owned();
        let cookie = cookie.cloned();

        move || {
            get_media_or_playlist_info(
                path,
                format!("ytsearch:{video_id}"),
                false,
                GET_INFO_TIMEOUT,
                &"1:1:1".parse().unwrap(),
                cookie.as_ref(),
            )
        }
    })
    .await
    .map_err(HandlerError::new)?
    {
        Ok(videos) => videos,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Error while get info");

            error::occured_in_chosen_inline_result(&bot, "Sorry, an error occurred while getting media info", inline_message_id, None)
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
                video,
                yt_dlp_cfg.max_file_size,
                yt_dlp_cfg.executable_path,
                &yt_toolkit_cfg.url,
                temp_dir.path(),
                true,
                DOWNLOAD_MEDIA_TIMEOUT,
                cookie,
                &PreferredLanguages::default()
                    .languages
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Box<[_]>>(),
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendVideo::new(chat_cfg.receiver_chat_id, InputFile::fs(path))
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
                        .caption(html_text_link("Link", format!("https://youtu.be/{video_id}")))
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
                    let _ = bot.send(DeleteMessage::new(chat_cfg.receiver_chat_id, message_id)).await;
                }
            });
        } else {
            let title = video.title.clone();

            #[allow(clippy::cast_possible_truncation)]
            let duration = video.duration.map(|duration| duration as i64);

            let AudioInFS { path, thumbnail_path } = download::audio_to_temp_dir(
                video,
                video_id,
                yt_dlp_cfg.max_file_size,
                &yt_dlp_cfg.executable_path,
                &yt_toolkit_cfg.url,
                temp_dir.path(),
                true,
                DOWNLOAD_MEDIA_TIMEOUT,
                cookie,
                &PreferredLanguages::default()
                    .languages
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Box<[_]>>(),
            )
            .await?;

            let message = send::with_retries(
                &bot,
                SendAudio::new(chat_cfg.receiver_chat_id, InputFile::fs(path))
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
                        .caption(html_text_link("Link", format!("https://youtu.be/{video_id}")))
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
                    let _ = bot.send(DeleteMessage::new(chat_cfg.receiver_chat_id, message_id)).await;
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

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, url))]
pub async fn media_select_inline_query(
    bot: Bot,
    InlineQuery {
        id: query_id, query: url, ..
    }: InlineQuery,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
    Extension(cookies): Extension<Cookies>,
) -> HandlerResult {
    Span::current().record("query_id", query_id.as_ref());
    Span::current().record("url", url.as_ref());

    let Ok(url) = Url::parse(&url) else {
        error::occured_in_inline_query_occured(&bot, &query_id, "Sorry, video not found").await?;
        return Ok(EventReturn::Finish);
    };

    event!(Level::DEBUG, "Got url");

    let videos: Vec<ShortInfo> = match get_video_info(Client::new(), &yt_toolkit_cfg.url, url.as_str()).await {
        Ok(videos) => videos.into_iter().map(Into::into).collect(),
        Err(err) => {
            if let GetVideoInfoErrorKind::GetVideoId(err) = err {
                event!(Level::ERROR, %err, "Unsupported URL for YT Toolkit");
            } else {
                event!(Level::ERROR, err = format_error_report(&err), "Getting media info YT Toolkit error");
            }

            match spawn_blocking(move || {
                let cookie = cookies.get_path_by_optional_host(url.host().as_ref()).cloned();

                get_media_or_playlist_info(
                    &yt_dlp_cfg.executable_path,
                    url,
                    true,
                    GET_MEDIA_OR_PLAYLIST_INFO_INLINE_QUERY_TIMEOUT,
                    &"1:1:1".parse().unwrap(),
                    cookie.as_ref(),
                )
            })
            .await
            .map_err(HandlerError::new)?
            {
                Ok(videos) => videos.map(Into::into).collect(),
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Getting media info error");

                    error::occured_in_inline_query_occured(&bot, &query_id, "Sorry, an error occurred while getting media info").await?;
                    return Ok(EventReturn::Finish);
                }
            }
        }
    };

    let videos_len = videos.len();

    if videos_len == 0 {
        event!(Level::WARN, "Playlist empty");

        error::occured_in_inline_query_occured(&bot, &query_id, "Playlist empty").await?;
        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, videos_len, "Got media info");

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos_len);

    for video in videos {
        let title = video.title.as_deref().unwrap_or("Untitled");
        let title_html = html_code(html_quote(title));
        let thumbnail_url = video.thumbnail();
        let result_id = Uuid::new_v4();

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{result_id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
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
                "↑",
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
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

#[instrument(skip_all, fields(query_id, text))]
pub async fn media_search_inline_query(
    bot: Bot,
    InlineQuery {
        id: query_id, query: text, ..
    }: InlineQuery,
    Extension(yt_toolkit_cfg): Extension<YtToolkitConfig>,
) -> HandlerResult {
    Span::current().record("query_id", query_id.as_ref());
    Span::current().record("text", text.as_ref());

    event!(Level::DEBUG, "Got text");

    let videos: Vec<ShortInfo> = match search_video(Client::new(), &yt_toolkit_cfg.url, &text).await {
        Ok(videos) => videos
            .into_iter()
            .map(Into::into)
            .enumerate()
            .filter(|(index, _)| *index < 25)
            .map(|(_, video)| video)
            .collect(),
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Search media error");

            error::occured_in_inline_query_occured(&bot, &query_id, "Sorry, an error occurred while getting media info").await?;
            return Ok(EventReturn::Finish);
        }
    };

    let videos_len = videos.len();

    if videos_len == 0 {
        event!(Level::WARN, "Result empty");

        error::occured_in_inline_query_occured(&bot, &query_id, "Result empty").await?;
        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, videos_len, "Got media info");

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos_len);

    for video in videos {
        let title = video.title.as_deref().unwrap_or("Untitled");
        let title_html = html_code(html_quote(title));
        let thumbnail_url = video.thumbnail();

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{}", &video.id),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail_url)
            .description("Click to download video")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading...").callback_data("video_download")
            ]]))
            .into(),
        );
        results.push(
            InlineQueryResultArticle::new(
                format!("audio_{}", &video.id),
                "↑",
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
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
