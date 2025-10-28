use crate::{
    config::{ChatConfig, YtDlpConfig},
    entities::{AudioAndFormat, PreferredLanguages, Range, TgAudioInPlaylist, UrlWithParams},
    handlers_utils::error,
    interactors::{
        download::{DownloadAudio, DownloadAudioInput, DownloadAudioPlaylist, DownloadAudioPlaylistInput},
        send_media::{SendAudioInFS, SendAudioInFSInput, SendAudioPlaylistById, SendAudioPlaylistByIdInput},
        GetMedaInfoByURLInput, GetMediaInfoByURL, Interactor as _,
    },
    utils::{format_error_report, FormatErrorToMessage as _},
};

use froodi::Inject;
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
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_media_info): Inject<GetMediaInfoByURL>,
    Inject(download): Inject<DownloadAudio>,
    Inject(download_playlist): Inject<DownloadAudioPlaylist>,
    Inject(send_media_in_fs): Inject<SendAudioInFS>,
    Inject(send_playlist): Inject<SendAudioPlaylistById>,
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
        let audio_and_format = match AudioAndFormat::new_with_select_format(&video, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
        let audio_in_fs = match download.execute(DownloadAudioInput::new(&url, audio_and_format)).await {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Download audio err");
                let text = format!(
                    "Sorry, an error to download the audio\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        let _file_id = match send_media_in_fs
            .execute(SendAudioInFSInput::new(
                chat_id,
                Some(message_id),
                audio_in_fs,
                video.title.as_deref().unwrap_or(video.id.as_ref()),
                video.title.as_deref(),
                video.uploader.as_deref(),
                #[allow(clippy::cast_possible_truncation)]
                video.duration.map(|duration| duration as i64),
                false,
            ))
            .await
        {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Send audio err");
                let text = format!(
                    "Sorry, an error to send the audio\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        return Ok(EventReturn::Finish);
    }

    let mut media_and_formats = Vec::with_capacity(videos.len());
    for video in videos.iter() {
        media_and_formats.push(
            match AudioAndFormat::new_with_select_format(video, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
    let (download_playlist_input, mut video_in_fs_receiver) = DownloadAudioPlaylistInput::new(&url, media_and_formats);
    let download_playlist_handle = tokio::spawn({
        let bot = bot.clone();
        let videos = videos.clone();
        async move {
            let mut playlist = vec![];
            let mut errs = vec![];
            while let Some((index, res)) = video_in_fs_receiver.recv().await {
                let audio_in_fs = match res {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, %err, "Download video err");
                        errs.push(err.format(&bot.token));
                        continue;
                    }
                };
                let video = videos.get(index).unwrap();

                let file_id = match send_media_in_fs
                    .execute(SendAudioInFSInput::new(
                        chat_cfg.receiver_chat_id,
                        Some(message_id),
                        audio_in_fs,
                        video.title.as_deref().unwrap_or(video.id.as_ref()),
                        video.title.as_deref(),
                        video.uploader.as_deref(),
                        #[allow(clippy::cast_possible_truncation)]
                        video.duration.map(|duration| duration as i64),
                        true,
                    ))
                    .await
                {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Send audio err");
                        errs.push(err.format(&bot.token));
                        continue;
                    }
                };
                playlist.push(TgAudioInPlaylist::new(file_id, index))
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
        let mut text = "Sorry, some audio download/send failed:\n".to_owned();
        for (index, err) in errors.into_iter().enumerate() {
            text.push_str(&expandable_blockquote(format!("{index}. {err}")));
            text.push('\n');
        }
        if let Err(err) = error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await {
            event!(Level::ERROR, %err);
        }
    }
    if let Err(err) = send_playlist
        .execute(SendAudioPlaylistByIdInput::new(chat_id, Some(message_id), playlist))
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
