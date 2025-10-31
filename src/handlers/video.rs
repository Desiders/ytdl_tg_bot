use crate::{
    config::{ChatConfig, YtDlpConfig},
    database::TxManager,
    entities::{PreferredLanguages, Range, TgVideoInPlaylist, UrlWithParams, VideoAndFormat},
    handlers_utils::error,
    interactors::{
        download::{DownloadVideo, DownloadVideoInput, DownloadVideoPlaylist, DownloadVideoPlaylistInput},
        send_media::{
            SendVideoById, SendVideoByIdInput, SendVideoInFS, SendVideoInFSInput, SendVideoPlaylistById, SendVideoPlaylistByIdInput,
        },
        GetVideoByURL, GetVideoByURLInput,
        GetVideoByURLKind::{Empty, PlaylistCached, PlaylistUncached, SingleCached, SingleUncached},
        Interactor as _,
    },
    utils::{format_error_report, FormatErrorToMessage as _},
};

use froodi::{Inject, InjectTransient};
use std::str::FromStr as _;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    utils::text::html_formatter::expandable_blockquote,
    Bot, Extension,
};
use tracing::{event, instrument, Level};

#[instrument(skip_all, fields(%message_id = message.id(), %chat_id = message.chat().id(), %url = url.as_str(), ?params))]
pub async fn download(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_media): Inject<GetVideoByURL>,
    Inject(download): Inject<DownloadVideo>,
    Inject(download_playlist): Inject<DownloadVideoPlaylist>,
    Inject(send_media_in_fs): Inject<SendVideoInFS>,
    Inject(send_media_by_id): Inject<SendVideoById>,
    Inject(send_playlist): Inject<SendVideoPlaylistById>,
    InjectTransient(tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    event!(Level::DEBUG, "Got url");

    let message_id = message.id();
    let chat_id = message.chat().id();

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
    match get_media.execute(GetVideoByURLInput::new(&url, &range, tx_manager)).await {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = send_media_by_id
                .execute(SendVideoByIdInput::new(chat_id, Some(message_id), &file_id))
                .await
            {
                event!(Level::ERROR, err = format_error_report(&err), "Send err");
                let text = format!("Sorry, an error to send media\n{}", expandable_blockquote(err.format(&bot.token)));
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
            }
        }
        Ok(PlaylistCached(playlist)) => {
            if let Err(err) = send_playlist
                .execute(SendVideoPlaylistByIdInput::new(chat_id, Some(message_id), playlist))
                .await
            {
                event!(Level::ERROR, err = format_error_report(&err), "Send err");
                let text = format!(
                    "Sorry, an error to send playlist\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
            }
        }
        Ok(SingleUncached(media)) => {
            let media_and_format = match VideoAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
            let media_in_fs = match download.execute(DownloadVideoInput::new(&url, media_and_format)).await {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Download err");
                    let text = format!(
                        "Sorry, an error to download media\n{}",
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
                    media_in_fs,
                    media.title.as_deref().unwrap_or(media.id.as_ref()),
                    media.width,
                    media.height,
                    #[allow(clippy::cast_possible_truncation)]
                    media.duration.map(|duration| duration as i64),
                    false,
                ))
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Send err");
                    let text = format!("Sorry, an error to send media\n{}", expandable_blockquote(err.format(&bot.token)));
                    error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                    return Ok(EventReturn::Finish);
                }
            };
        }
        Ok(PlaylistUncached(videos)) => {
            let mut media_and_formats = vec![];
            for media in videos.iter() {
                media_and_formats.push(
                    match VideoAndFormat::new_with_select_format(media, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
                        let media_in_fs = match res {
                            Ok(val) => val,
                            Err(err) => {
                                event!(Level::ERROR, %err, "Download err");
                                errs.push(err.format(&bot.token));
                                continue;
                            }
                        };
                        let media = videos.get(index).unwrap();
                        let file_id = match send_media_in_fs
                            .execute(SendVideoInFSInput::new(
                                chat_cfg.receiver_chat_id,
                                Some(message_id),
                                media_in_fs,
                                media.title.as_deref().unwrap_or(media.id.as_ref()),
                                media.width,
                                media.height,
                                #[allow(clippy::cast_possible_truncation)]
                                media.duration.map(|duration| duration as i64),
                                true,
                            ))
                            .await
                        {
                            Ok(val) => val,
                            Err(err) => {
                                event!(Level::ERROR, err = format_error_report(&err), "Send err");
                                errs.push(err.format(&bot.token));
                                continue;
                            }
                        };
                        playlist.push(TgVideoInPlaylist::new(file_id, index))
                    }
                    (playlist, errs)
                }
            });
            if let Err(err) = download_playlist.execute(download_playlist_input).await {
                event!(Level::ERROR, %err, "Download err");
                let text = format!(
                    "Sorry, an error to download playlist\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                if let Err(err) = error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await {
                    event!(Level::ERROR, %err);
                }
            }
            let (playlist, errors) = download_playlist_handle.await.unwrap();
            if !errors.is_empty() {
                let mut text = "Sorry, some download/send media failed:\n".to_owned();
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
                event!(Level::ERROR, %err, "Send err");
                let text = format!(
                    "Sorry, an error to send playlist\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                if let Err(err) = error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await {
                    event!(Level::ERROR, %err);
                }
            }
        }
        Ok(Empty) => {
            event!(Level::WARN, "Empty playlist");
            let text = format!("Playlist is empty");
            error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
        }
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Get err");
            let text = format!("Sorry, an error to get info\n{}", expandable_blockquote(err.format(&bot.token)));
            error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
    };
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.id(), %chat_id = message.chat().id(), %url = url.as_str(), ?params))]
pub async fn download_quite(
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_media): Inject<GetVideoByURL>,
    Inject(download): Inject<DownloadVideo>,
    Inject(download_playlist): Inject<DownloadVideoPlaylist>,
    Inject(send_media_in_fs): Inject<SendVideoInFS>,
    Inject(send_media_by_id): Inject<SendVideoById>,
    Inject(send_playlist): Inject<SendVideoPlaylistById>,
    InjectTransient(tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    event!(Level::DEBUG, "Got url");

    let message_id = message.id();
    let chat_id = message.chat().id();

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
    match get_media.execute(GetVideoByURLInput::new(&url, &range, tx_manager)).await {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = send_media_by_id
                .execute(SendVideoByIdInput::new(chat_id, Some(message_id), &file_id))
                .await
            {
                event!(Level::ERROR, %err, "Send err");
            }
        }
        Ok(PlaylistCached(playlist)) => {
            if let Err(err) = send_playlist
                .execute(SendVideoPlaylistByIdInput::new(chat_id, Some(message_id), playlist))
                .await
            {
                event!(Level::ERROR, %err, "Send playlist err");
            }
        }
        Ok(SingleUncached(media)) => {
            let media_and_format = match VideoAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages) {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, %err, "Select format err");
                    return Ok(EventReturn::Finish);
                }
            };
            let media_in_fs = match download.execute(DownloadVideoInput::new(&url, media_and_format)).await {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Download err");
                    return Ok(EventReturn::Finish);
                }
            };
            let _file_id = match send_media_in_fs
                .execute(SendVideoInFSInput::new(
                    chat_id,
                    Some(message_id),
                    media_in_fs,
                    media.title.as_deref().unwrap_or(media.id.as_ref()),
                    media.width,
                    media.height,
                    #[allow(clippy::cast_possible_truncation)]
                    media.duration.map(|duration| duration as i64),
                    false,
                ))
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Send err");
                    return Ok(EventReturn::Finish);
                }
            };
        }
        Ok(PlaylistUncached(videos)) => {
            let mut media_and_formats = vec![];
            for media in videos.iter() {
                media_and_formats.push(
                    match VideoAndFormat::new_with_select_format(media, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
                        let media_in_fs = match res {
                            Ok(val) => val,
                            Err(err) => {
                                event!(Level::ERROR, %err, "Download err");
                                continue;
                            }
                        };
                        let media = videos.get(index).unwrap();
                        let file_id = match send_media_in_fs
                            .execute(SendVideoInFSInput::new(
                                chat_cfg.receiver_chat_id,
                                Some(message_id),
                                media_in_fs,
                                media.title.as_deref().unwrap_or(media.id.as_ref()),
                                media.width,
                                media.height,
                                #[allow(clippy::cast_possible_truncation)]
                                media.duration.map(|duration| duration as i64),
                                true,
                            ))
                            .await
                        {
                            Ok(val) => val,
                            Err(err) => {
                                event!(Level::ERROR, err = format_error_report(&err), "Send err");
                                continue;
                            }
                        };
                        playlist.push(TgVideoInPlaylist::new(file_id, index))
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
        }
        Ok(Empty) => {
            event!(Level::WARN, "Empty playlist");
        }
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Get err");
        }
    };
    Ok(EventReturn::Finish)
}
