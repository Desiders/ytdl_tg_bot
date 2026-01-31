use crate::{
    config::Config,
    database::TxManager,
    entities::{language::Language, Params, Range},
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
use std::str::FromStr as _;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    types::ChosenInlineResult,
    utils::text::{html_expandable_blockquote, html_quote, html_text_link},
    Bot, Extension,
};
use tracing::{debug, error, instrument, warn, Span};
use url::Url;

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_video(
    bot: Bot,
    params: Params,
    url_option: Option<Extension<Url>>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(cfg): Inject<Config>,
    Inject(get_media): Inject<get_media::GetVideoByURL>,
    Inject(download_media): Inject<media::DownloadVideo>,
    Inject(send_media_in_fs): Inject<send_media::fs::SendVideo>,
    Inject(edit_media_by_id): Inject<send_media::id::EditVideo>,
    Inject(add_downloaded_media): Inject<downloaded_media::AddVideo>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = match url_option {
        Some(Extension(val)) => val,
        None => {
            let (_, video_id) = result_id.split_once('_').expect("incorrect inline message ID");
            Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).unwrap()
        }
    };

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("url", url.as_str());

    debug!("Got url");

    let playlist_range = Range::default();
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
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    caption: &html_text_link("Link", url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit err");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
            }
        }
        Ok(Playlist { mut cached, .. }) if cached.len() > 0 => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    caption: &html_text_link("Link", url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit err");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if uncached.len() > 0 => {
            let mut errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (input, mut err_receiver, mut progress_receiver) = media::DownloadMediaInput::new_with_progress(&url, &media, formats);

            let ((), (), download_res) = tokio::join!(
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if let Err(_) = progress::is_downloading_with_progress_in_chosen_inline(&bot, inline_message_id, progress_str).await
                        {
                            break;
                        };
                    }
                },
                async {
                    while let Some(err) = err_receiver.recv().await {
                        errs.push(html_quote(err.format(&bot.token)));
                    }
                },
                async {
                    let _ = progress::is_downloading_in_chosen_inline(&bot, inline_message_id).await;
                    download_media.execute(input).await
                }
            );
            let (media_in_fs, format) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(&bot, inline_message_id, &errs, Some(ParseMode::HTML)).await;
                    return Ok(EventReturn::Finish);
                }
                Err(err) => {
                    error!(%err, "Download err");
                    let _ = progress::is_error_in_chosen_inline(
                        &bot,
                        inline_message_id,
                        &html_quote(err.format(&bot.token)),
                        Some(ParseMode::HTML),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            let _ = progress::is_sending_in_chosen_inline(&bot, inline_message_id).await;
            let file_id = match send_media_in_fs
                .execute(send_media::fs::SendVideoInput {
                    chat_id: cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_in_fs,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    width: format.width,
                    height: format.height,
                    duration: media.duration.map(|val| val as i64),
                    with_delete: true,
                })
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = format_error_report(&err), "Send err");
                    let _ = progress::is_error_in_chosen_inline(
                        &bot,
                        inline_message_id,
                        &html_quote(err.format(&bot.token)),
                        Some(ParseMode::HTML),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    caption: &html_text_link("Link", &url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit err");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
                return Ok(EventReturn::Finish);
            }

            if let Err(err) = add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id: file_id.into(),
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: audio_language.clone(),
                    tx_manager: &mut tx_manager,
                })
                .await
            {
                error!(%err, "Add err");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let text = "Playlist is empty";
            let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get err");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(err.format(&bot.token)))
            );
            let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
            return Ok(EventReturn::Finish);
        }
        _ => unreachable!("Incorrect branch"),
    }
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_audio(
    bot: Bot,
    params: Params,
    url_option: Option<Extension<Url>>,
    ChosenInlineResult { inline_message_id, .. }: ChosenInlineResult,
    Inject(cfg): Inject<Config>,
    Inject(get_media): Inject<get_media::GetAudioByURL>,
    Inject(download_media): Inject<media::DownloadAudio>,
    Inject(send_media_in_fs): Inject<send_media::fs::SendAudio>,
    Inject(edit_media_by_id): Inject<send_media::id::EditAudio>,
    Inject(add_downloaded_media): Inject<downloaded_media::AddAudio>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = match url_option {
        Some(Extension(val)) => val,
        None => {
            let (_, video_id) = inline_message_id.split_once('_').expect("incorrect inline message ID");
            Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).unwrap()
        }
    };

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("url", url.as_str());

    debug!("Got url");

    let playlist_range = Range::default();
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
            tx_manager: &mut tx_manager,
        })
        .await
    {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    caption: &html_text_link("Link", url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit err");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
            }
        }
        Ok(Playlist { mut cached, .. }) if cached.len() > 0 => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    caption: &html_text_link("Link", url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit err");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if uncached.len() > 0 => {
            let mut download_errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (input, mut err_receiver, mut progress_receiver) = media::DownloadMediaInput::new_with_progress(&url, &media, formats);

            let ((), (), download_res) = tokio::join!(
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if let Err(_) = progress::is_downloading_with_progress_in_chosen_inline(&bot, inline_message_id, progress_str).await
                        {
                            break;
                        };
                    }
                },
                async {
                    while let Some(err) = err_receiver.recv().await {
                        download_errs.push(html_quote(err.format(&bot.token)));
                    }
                },
                async {
                    let _ = progress::is_downloading_in_chosen_inline(&bot, inline_message_id).await;
                    download_media.execute(input).await
                }
            );
            let (media_in_fs, _format) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(&bot, inline_message_id, &download_errs, Some(ParseMode::HTML)).await;
                    return Ok(EventReturn::Finish);
                }
                Err(err) => {
                    error!(%err, "Download err");
                    let _ = progress::is_error_in_chosen_inline(
                        &bot,
                        inline_message_id,
                        &html_quote(err.format(&bot.token)),
                        Some(ParseMode::HTML),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            let _ = progress::is_sending_in_chosen_inline(&bot, inline_message_id).await;
            let file_id = match send_media_in_fs
                .execute(send_media::fs::SendAudioInput {
                    chat_id: cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_in_fs,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    title: media.title.as_deref(),
                    performer: media.uploader.as_deref(),
                    duration: media.duration.map(|val| val as i64),
                    with_delete: true,
                })
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = format_error_report(&err), "Send err");
                    let _ = progress::is_error_in_chosen_inline(
                        &bot,
                        inline_message_id,
                        &html_quote(err.format(&bot.token)),
                        Some(ParseMode::HTML),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    caption: &html_text_link("Link", &url),
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit err");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(err.format(&bot.token)))
                );
                let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
                return Ok(EventReturn::Finish);
            }

            if let Err(err) = add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id: file_id.into(),
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: audio_language.clone(),
                    tx_manager: &mut tx_manager,
                })
                .await
            {
                error!(%err, "Add err");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let text = "Playlist is empty";
            let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get err");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(err.format(&bot.token)))
            );
            let _ = progress::is_error_in_chosen_inline(&bot, inline_message_id, &text, Some(ParseMode::HTML)).await;
            return Ok(EventReturn::Finish);
        }
        _ => unreachable!("Incorrect branch"),
    }
    Ok(EventReturn::Finish)
}
