use crate::{
    config::{ChatConfig, YtDlpConfig},
    database::TxManager,
    entities::{AudioAndFormat, PreferredLanguages, Range, TgAudioInPlaylist, UrlWithParams},
    handlers_utils::error,
    interactors::{
        download::{DownloadAudioPlaylist, DownloadAudioPlaylistInput},
        send_media::{
            SendAudioById, SendAudioByIdInput, SendAudioInFS, SendAudioInFSInput, SendAudioPlaylistById, SendAudioPlaylistByIdInput,
        },
        AddDownloadedAudio, AddDownloadedMediaInput, GetAudioByURL, GetAudioByURLInput,
        GetAudioByURLKind::{Empty, Playlist, SingleCached},
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

#[instrument(skip_all, fields(%message_id = message.id(), %url = url.as_str(), ?params))]
pub async fn download(
    bot: Bot,
    message: Message,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_media): Inject<GetAudioByURL>,
    Inject(download_playlist): Inject<DownloadAudioPlaylist>,
    Inject(send_media_in_fs): Inject<SendAudioInFS>,
    Inject(send_media_by_id): Inject<SendAudioById>,
    Inject(send_playlist): Inject<SendAudioPlaylistById>,
    Inject(add_downloaded_media): Inject<AddDownloadedAudio>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
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
    match get_media
        .execute(GetAudioByURLInput::new(
            &url,
            &range,
            url.as_str(),
            url.domain().as_deref(),
            &mut tx_manager,
        ))
        .await
    {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = send_media_by_id
                .execute(SendAudioByIdInput::new(chat_id, Some(message_id), &file_id))
                .await
            {
                event!(Level::ERROR, %err, "Send err");
                let text = format!("Sorry, an error to send media\n{}", expandable_blockquote(err.format(&bot.token)));
                error::occured_in_message(&bot, chat_id, message_id, &text, Some(ParseMode::HTML)).await?;
            }
        }
        Ok(Playlist((cached, uncached))) => {
            let mut media_and_formats = vec![];
            for media in uncached.iter() {
                media_and_formats.push(
                    match AudioAndFormat::new_with_select_format(media, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
                let uncached = uncached.clone();
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
                        let media = uncached.get(index).unwrap();
                        let file_id = match send_media_in_fs
                            .execute(SendAudioInFSInput::new(
                                chat_cfg.receiver_chat_id,
                                Some(message_id),
                                media_in_fs,
                                media.title.as_deref().unwrap_or(media.id.as_ref()),
                                media.title.as_deref(),
                                media.uploader.as_deref(),
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
                        playlist.push(TgAudioInPlaylist::new(file_id.clone(), index));
                        if let Err(err) = add_downloaded_media
                            .execute(AddDownloadedMediaInput::new(
                                file_id.into(),
                                media.id.clone(),
                                media.display_id.clone(),
                                media.domain(),
                                &mut tx_manager,
                            ))
                            .await
                        {
                            event!(Level::ERROR, %err, "Add err");
                        }
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
                .execute(SendAudioPlaylistByIdInput::new(
                    chat_id,
                    Some(message_id),
                    playlist.into_iter().chain(cached).collect(),
                ))
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
