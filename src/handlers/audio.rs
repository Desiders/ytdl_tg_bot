use crate::{
    config::Config,
    database::TxManager,
    entities::{language::Language, Domains, MediaInPlaylist, Params, Range, Sections},
    handlers_utils::progress,
    interactors::{
        download::media,
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{Empty, Playlist, SingleCached},
        },
        send_media, Interactor as _,
    },
    utils::{format_error_report, FormatErrorToMessage as _},
};

use froodi::{Inject, InjectTransient};
use std::{
    str::FromStr as _,
    sync::atomic::{AtomicUsize, Ordering},
};
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    types::Message,
    utils::text::{html_expandable_blockquote, html_quote},
    Bot, Extension,
};
use tracing::{debug, error, instrument, warn};
use url::Url;

#[instrument(skip_all, fields(%message_id = message.id(), %url = url.as_str(), ?params))]
pub async fn download(
    bot: Bot,
    message: Message,
    params: Params,
    Extension(url): Extension<Url>,
    Inject(cfg): Inject<Config>,
    Inject(get_media): Inject<get_media::GetAudioByURL>,
    Inject(download_playlist): Inject<media::DownloadAudioPlaylist>,
    Inject(send_media_in_fs): Inject<send_media::fs::SendAudio>,
    Inject(send_media_by_id): Inject<send_media::id::SendAudio>,
    Inject(send_playlist): Inject<send_media::id::SendAudioPlaylist>,
    Inject(add_downloaded_media): Inject<downloaded_media::AddAudio>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    debug!("Got url");

    let message_id = message.id();
    let chat_id = message.chat().id();

    let progress_message = progress::new(&bot, "🔍 Preparing download...", chat_id, Some(message_id)).await?;
    let progress_message_id = progress_message.id();

    let playlist_range = match params.0.get("items") {
        Some(raw_value) => match Range::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(%err, "Parse range err");
                let text = format!(
                    "Sorry, an error to parse range\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_progress(&bot, chat_id, progress_message_id, &text, Some(ParseMode::HTML)).await;
                return Ok(EventReturn::Finish);
            }
        },
        None => Range::default(),
    };
    let sections = match params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(%err, "Parse sections err");
                let text = format!(
                    "Sorry, an error to parse sections\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_progress(&bot, chat_id, progress_message_id, &text, Some(ParseMode::HTML)).await;
                return Ok(EventReturn::Finish);
            }
        }),
        None => None,
    };
    let audio_language = match params.0.get("lang") {
        Some(raw_value) => Language::from_str(raw_value).unwrap(),
        None => Language::default(),
    };

    match get_media
        .execute(get_media::GetMediaByURLInput {
            url: &url,
            playlist_range: &playlist_range,
            cache_search: url.as_str(),
            domain: url.domain(),
            audio_language: &audio_language,
            sections: sections.as_ref(),
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = send_media_by_id
                .execute(send_media::id::SendMediaInput {
                    chat_id,
                    reply_to_message_id: Some(message_id),
                    id: &file_id,
                    webpage_url: Some(&url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Send err");
                let text = format!(
                    "Sorry, an error to send media\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_progress(&bot, chat_id, progress_message_id, &text, Some(ParseMode::HTML)).await;
            } else {
                let _ = progress::delete(&bot, chat_id, progress_message_id).await;
            }
        }
        Ok(Playlist { cached, uncached }) => {
            let mut send_err = None;
            let mut download_errs = vec![];
            let (cached_len, uncached_len) = (cached.len(), uncached.len());
            let mut downloaded_playlist = Vec::with_capacity(cached_len + uncached_len);
            downloaded_playlist.extend(cached);
            let (input, mut media_receiver, mut errs_receiver, mut progress_receiver) =
                media::DownloadMediaPlaylistInput::new_with_progress(&url, uncached, sections.as_ref());

            let downloaded_media_count = AtomicUsize::new(cached_len);
            tokio::join!(
                async {
                    while let Some((media_in_fs, media, _format)) = media_receiver.recv().await {
                        let _ = progress::is_sending(&bot, chat_id, progress_message_id).await;
                        let file_id = match send_media_in_fs
                            .execute(send_media::fs::SendAudioInput {
                                chat_id: cfg.chat.receiver_chat_id,
                                reply_to_message_id: Some(message_id),
                                media_in_fs,
                                name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                                title: media.title.as_deref(),
                                performer: media.uploader.as_deref(),
                                #[allow(clippy::cast_possible_truncation)]
                                duration: media.duration.map(|val| val as i64),
                                with_delete: true,
                                webpage_url: &media.webpage_url,
                            })
                            .await
                        {
                            Ok(val) => val,
                            Err(err) => {
                                error!(err = format_error_report(&err), "Send err");
                                send_err = Some(html_quote(err.format(&bot.token)));
                                continue;
                            }
                        };
                        downloaded_playlist.push(MediaInPlaylist {
                            file_id: file_id.clone().into(),
                            playlist_index: media.playlist_index,
                            webpage_url: Some(media.webpage_url.clone()),
                        });

                        if let Err(err) = add_downloaded_media
                            .execute(downloaded_media::AddMediaInput {
                                file_id: file_id.into(),
                                id: media.id.clone(),
                                display_id: media.display_id.clone(),
                                domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                                audio_language: audio_language.clone(),
                                sections: sections.clone(),
                                tx_manager: &mut tx_manager,
                            })
                            .await
                        {
                            error!(%err, "Add err");
                        }
                        downloaded_media_count.fetch_add(1, Ordering::SeqCst);
                    }
                },
                async {
                    while let Some(errs) = errs_receiver.recv().await {
                        download_errs.push(errs.into_iter().map(|err| html_quote(err.format(&bot.token))).collect());
                    }
                },
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if progress::is_downloading_with_progress(
                            &bot,
                            chat_id,
                            progress_message_id,
                            progress_str,
                            downloaded_media_count.load(Ordering::SeqCst),
                            cached_len + uncached_len,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        };
                    }
                },
                async {
                    if let Err(err) = download_playlist.execute(input).await {
                        error!(%err, "Download err");
                        let text = format!(
                            "Sorry, an error to download playlist\n{}",
                            html_expandable_blockquote(html_quote(err.format(&bot.token)))
                        );
                        let _ = progress::is_error_in_progress(&bot, chat_id, progress_message_id, &text, Some(ParseMode::HTML)).await;
                    }
                }
            );
            let errs = download_errs.into_iter().chain(send_err.map(|err| vec![err])).collect::<Vec<_>>();
            let media_to_send_count = downloaded_playlist.len();
            let _ = progress::is_errors_if_exist(&bot, chat_id, progress_message_id, &errs, media_to_send_count).await;

            downloaded_playlist.sort_by_key(|val| val.playlist_index);
            if let Err(err) = send_playlist
                .execute(send_media::id::SendPlaylistInput {
                    chat_id,
                    reply_to_message_id: Some(message_id),
                    playlist: downloaded_playlist,
                })
                .await
            {
                error!(%err, "Send err");
                let text = format!(
                    "Sorry, an error to send playlist\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_progress(&bot, chat_id, progress_message_id, &text, Some(ParseMode::HTML)).await;
            } else if errs.is_empty() {
                let _ = progress::delete(&bot, chat_id, progress_message_id).await;
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let text = "Playlist is empty";
            let _ = progress::is_error_in_progress(&bot, chat_id, message_id, text, Some(ParseMode::HTML)).await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get err");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(err.format(&bot.token)))
            );
            let _ = progress::is_error_in_progress(&bot, chat_id, progress_message_id, &text, Some(ParseMode::HTML)).await;
            return Ok(EventReturn::Finish);
        }
    }
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(%message_id = message.id()))]
pub async fn random(
    message: Message,
    params: Params,
    Inject(get_media): Inject<downloaded_media::GetRandomAudio>,
    Inject(send_playlist): Inject<send_media::id::SendAudioPlaylist>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let message_id = message.id();
    let chat_id = message.chat().id();

    let domains = params.0.get("domains").map(|val| Domains::from_str(val).unwrap());
    match get_media
        .execute(downloaded_media::GetRandomMediaInput {
            limit: 1,
            domains: domains.as_ref(),
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(playlist) => {
            if let Err(err) = send_playlist
                .execute(send_media::id::SendPlaylistInput {
                    chat_id,
                    reply_to_message_id: Some(message_id),
                    playlist: playlist
                        .into_iter()
                        .enumerate()
                        .map(|(index, media)| MediaInPlaylist {
                            file_id: media.file_id,
                            playlist_index: index.try_into().unwrap(),
                            webpage_url: None,
                        })
                        .collect(),
                })
                .await
            {
                error!(%err, "Send err");
            }
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get err");
        }
    }
    Ok(EventReturn::Finish)
}
