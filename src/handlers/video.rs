use crate::{
    config::{ChatConfig, YtDlpConfig},
    entities::{PreferredLanguages, Range, TgVideoInPlaylist, UrlWithParams, VideoAndFormat},
    handlers_utils::error,
    interactors::{
        download::{DownloadVideo, DownloadVideoInput, DownloadVideoPlaylist, DownloadVideoPlaylistInput},
        send_media::{SendVideoInFS, SendVideoInFSInput, SendVideoPlaylistById, SendVideoPlaylistByIdInput},
        GetMedaInfoByURLInput, GetMediaInfoByURL, Interactor as _,
    },
    utils::{format_error_report, FormatErrorToMessage as _},
};

use froodi::async_impl::Container;
use std::str::FromStr as _;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    utils::text::html_formatter::expandable_blockquote,
    Bot, Extension,
};
use tracing::{event, instrument, Level};

#[instrument(skip_all, fields(message_id = message.id(), chat_id = message.chat().id(), url = url.as_str(), ?params))]
pub async fn download(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(container): Extension<Container>,
) -> HandlerResult {
    event!(Level::DEBUG, "Got url");

    let message_id = message.id();
    let chat_id = message.chat().id();

    let mut get_media_info = container.get_transient::<GetMediaInfoByURL>().await.unwrap();
    let mut download = container.get_transient::<DownloadVideo>().await.unwrap();
    let mut download_playlist = container.get_transient::<DownloadVideoPlaylist>().await.unwrap();
    let mut send_media_in_fs = container.get_transient::<SendVideoInFS>().await.unwrap();
    let mut send_playlist = container.get_transient::<SendVideoPlaylistById>().await.unwrap();

    let range = match params.get("items") {
        Some(raw_value) => match Range::from_str(raw_value) {
            Ok(range) => range,
            Err(err) => {
                event!(Level::ERROR, %err, "Parse range err");
                let text = format!("Sorry, an error to parse range\n{}", expandable_blockquote(err.format(&bot.token)));
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        },
        None => Range::default(),
    };
    let preferred_languages = match params.get("lang") {
        Some(raw_value) => PreferredLanguages::from_str(raw_value).unwrap(),
        None => PreferredLanguages::default(),
    };
    let mut videos = match get_media_info.execute(GetMedaInfoByURLInput::new(&url, &range)).await {
        Ok(val) => val,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Get info err");
            let text = format!(
                "Sorry, an error to get media info\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
    };
    if videos.len() == 1 {
        let video = videos.remove(0);
        let video_and_format = match VideoAndFormat::new_with_select_format(&video, yt_dlp_cfg.max_file_size, &preferred_languages) {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, %err, "Select format err");
                let text = format!(
                    "Sorry, an error to select a format\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        let video_in_fs = match download.execute(DownloadVideoInput::new(&url, video_and_format)).await {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Download video err");
                let text = format!(
                    "Sorry, an error to download the video\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        let _file_id = match send_media_in_fs
            .execute(SendVideoInFSInput::new(
                chat_id,
                Some(message_id),
                video_in_fs,
                video.title.as_deref().unwrap_or(video.id.as_ref()),
                video.width,
                video.height,
                #[allow(clippy::cast_possible_truncation)]
                video.duration.map(|duration| duration as i64),
                false,
            ))
            .await
        {
            Ok((_message_id, file_id)) => file_id,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Send video err");
                let text = format!(
                    "Sorry, an error to send the video\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };

        return Ok(EventReturn::Finish);
    }

    let mut media_and_formats = vec![];
    for video in videos.iter() {
        media_and_formats.push(
            match VideoAndFormat::new_with_select_format(video, yt_dlp_cfg.max_file_size, &preferred_languages) {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, %err, "Select format err");
                    let text = format!(
                        "Sorry, an error to select a format\n{}",
                        expandable_blockquote(err.format(&bot.token))
                    );
                    error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                    return Ok(EventReturn::Finish);
                }
            },
        );
    }
    let (download_playlist_input, mut video_in_fs_receiver) = DownloadVideoPlaylistInput::new(&url, media_and_formats);
    let download_playlist_handle = tokio::spawn({
        let bot = bot.clone();
        let videos = videos.clone();
        async move {
            let mut playlist = vec![];
            let mut errs = vec![];
            while let Some((index, res)) = video_in_fs_receiver.recv().await {
                let video_in_fs = match res {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, %err, "Download video err");
                        errs.push(err.format(&bot.token));
                        continue;
                    }
                };
                let video = videos.get(index).unwrap();

                match send_media_in_fs
                    .execute(SendVideoInFSInput::new(
                        chat_cfg.receiver_chat_id,
                        Some(message_id),
                        video_in_fs,
                        video.title.as_deref().unwrap_or(video.id.as_ref()),
                        video.width,
                        video.height,
                        #[allow(clippy::cast_possible_truncation)]
                        video.duration.map(|duration| duration as i64),
                        true,
                    ))
                    .await
                {
                    Ok((_, file_id)) => {
                        playlist.push(TgVideoInPlaylist::new(file_id, index));
                    }
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Send video err");
                        errs.push(err.format(&bot.token));
                    }
                }
            }
            (playlist, errs)
        }
    });

    if let Err(err) = download_playlist.execute(download_playlist_input).await {
        event!(Level::ERROR, %err, "Download playlist err");
        let text = format!(
            "Sorry, an error to download a playlist\n{}",
            expandable_blockquote(err.format(&bot.token))
        );
        if let Err(err) = error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await {
            event!(Level::ERROR, %err);
        }
    }
    let (playlist, errors) = download_playlist_handle.await.unwrap();
    if !errors.is_empty() {
        let mut text = "Sorry, some video download/send failed:\n".to_owned();
        for (index, err) in errors.into_iter().enumerate() {
            text.push_str(&expandable_blockquote(format!("{index}. {err}")));
            text.push('\n');
        }
        if let Err(err) = error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await {
            event!(Level::ERROR, %err);
        }
    }
    if let Err(err) = send_playlist
        .execute(SendVideoPlaylistByIdInput::new(chat_id, Some(message_id), playlist))
        .await
    {
        event!(Level::ERROR, %err, "Send playlist err");
        let text = format!(
            "Sorry, an error to send the playlist\n{}",
            expandable_blockquote(err.format(&bot.token))
        );
        if let Err(err) = error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await {
            event!(Level::ERROR, %err);
        }
    }

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(message_id = message.id(), chat_id = message.chat().id(), url = url.as_str(), ?params))]
pub async fn download_quite(
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(container): Extension<Container>,
) -> HandlerResult {
    event!(Level::DEBUG, "Got url");

    let message_id = message.id();
    let chat_id = message.chat().id();

    let mut get_media_info = container.get_transient::<GetMediaInfoByURL>().await.unwrap();
    let mut download = container.get_transient::<DownloadVideo>().await.unwrap();
    let mut download_playlist = container.get_transient::<DownloadVideoPlaylist>().await.unwrap();
    let mut send_media_in_fs = container.get_transient::<SendVideoInFS>().await.unwrap();
    let mut send_playlist = container.get_transient::<SendVideoPlaylistById>().await.unwrap();

    let range = match params.get("items") {
        Some(raw_value) => match Range::from_str(raw_value) {
            Ok(range) => range,
            Err(err) => {
                event!(Level::ERROR, %err, "Parse range err");
                return Ok(EventReturn::Finish);
            }
        },
        None => Range::default(),
    };
    let preferred_languages = match params.get("lang") {
        Some(raw_value) => PreferredLanguages::from_str(raw_value).unwrap(),
        None => PreferredLanguages::default(),
    };
    let mut videos = match get_media_info.execute(GetMedaInfoByURLInput::new(&url, &range)).await {
        Ok(val) => val,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Get info err");
            return Ok(EventReturn::Finish);
        }
    };
    if videos.len() == 1 {
        let video = videos.remove(0);
        let video_and_format = match VideoAndFormat::new_with_select_format(&video, yt_dlp_cfg.max_file_size, &preferred_languages) {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, %err, "Select format err");
                return Ok(EventReturn::Finish);
            }
        };
        let video_in_fs = match download.execute(DownloadVideoInput::new(&url, video_and_format)).await {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Download video err");
                return Ok(EventReturn::Finish);
            }
        };
        let _file_id = match send_media_in_fs
            .execute(SendVideoInFSInput::new(
                chat_id,
                Some(message_id),
                video_in_fs,
                video.title.as_deref().unwrap_or(video.id.as_ref()),
                video.width,
                video.height,
                #[allow(clippy::cast_possible_truncation)]
                video.duration.map(|duration| duration as i64),
                false,
            ))
            .await
        {
            Ok((_message_id, file_id)) => file_id,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Send video err");
                return Ok(EventReturn::Finish);
            }
        };

        return Ok(EventReturn::Finish);
    }

    let mut media_and_formats = vec![];
    for video in videos.iter() {
        media_and_formats.push(
            match VideoAndFormat::new_with_select_format(video, yt_dlp_cfg.max_file_size, &preferred_languages) {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, %err, "Select format err");
                    return Ok(EventReturn::Finish);
                }
            },
        );
    }
    let (download_playlist_input, mut video_in_fs_receiver) = DownloadVideoPlaylistInput::new(&url, media_and_formats);
    let download_playlist_handle = tokio::spawn({
        let videos = videos.clone();
        async move {
            let mut playlist = vec![];
            while let Some((index, res)) = video_in_fs_receiver.recv().await {
                let video_in_fs = match res {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, %err, "Download video err");
                        continue;
                    }
                };
                let video = videos.get(index).unwrap();

                match send_media_in_fs
                    .execute(SendVideoInFSInput::new(
                        chat_cfg.receiver_chat_id,
                        Some(message_id),
                        video_in_fs,
                        video.title.as_deref().unwrap_or(video.id.as_ref()),
                        video.width,
                        video.height,
                        #[allow(clippy::cast_possible_truncation)]
                        video.duration.map(|duration| duration as i64),
                        true,
                    ))
                    .await
                {
                    Ok((_, file_id)) => {
                        playlist.push(TgVideoInPlaylist::new(file_id, index));
                    }
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Send video err");
                    }
                }
            }
            playlist
        }
    });

    if let Err(err) = download_playlist.execute(download_playlist_input).await {
        event!(Level::ERROR, %err, "Download playlist err");
    }
    let playlist = download_playlist_handle.await.unwrap();
    if let Err(err) = send_playlist
        .execute(SendVideoPlaylistByIdInput::new(chat_id, Some(message_id), playlist))
        .await
    {
        event!(Level::ERROR, %err, "Send playlist err");
    }

    Ok(EventReturn::Finish)
}
