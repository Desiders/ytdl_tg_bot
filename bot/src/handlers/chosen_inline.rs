use crate::{
    config::Config,
    database::TxManager,
    entities::{language::Language, ChatConfig, Params, Range, Sections},
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
    services::messenger::{MessengerPort, TextFormat},
    utils::{format_error_report, ErrorMessageFormatter},
};

use froodi::{Inject, InjectTransient};
use std::str::FromStr as _;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::ChosenInlineResult,
    utils::text::{html_expandable_blockquote, html_quote},
    Extension,
};
use tracing::{debug, error, instrument, warn, Span};
use url::Url;

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_video<Messenger>(
    params: Params,
    url_option: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(cfg): Inject<Config>,
    Inject(error_formatter): Inject<ErrorMessageFormatter>,
    Inject(messenger): Inject<Messenger>,
    Inject(get_media): Inject<get_media::GetVideoByURL>,
    Inject(download_media): Inject<media::DownloadVideo>,
    Inject(upload_media): Inject<send_media::upload::SendVideo<Messenger>>,
    Inject(edit_media_by_id): Inject<send_media::id::EditVideo<Messenger>>,
    Inject(add_downloaded_media): Inject<downloaded_media::AddVideo>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = if let Some(Extension(val)) = url_option {
        val
    } else {
        let (_, video_id) = result_id.split_once('_').expect("Incorrect inline message ID");
        Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).unwrap()
    };

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("url", url.as_str());

    debug!("Got url");

    let playlist_range = Range::default();
    let sections = match params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(%err, "Parse sections error");
                let text = format!(
                    "Sorry, an error to parse sections\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
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
            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let mut errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (input, mut err_receiver, mut progress_receiver) =
                media::DownloadMediaInput::new_with_progress(&url, &media, sections.as_ref(), formats);

            let ((), (), download_res) = tokio::join!(
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if progress::is_downloading_with_progress_in_chosen_inline(&*messenger, inline_message_id, progress_str)
                            .await
                            .is_err()
                        {
                            break;
                        };
                    }
                },
                async {
                    while let Some(err) = err_receiver.recv().await {
                        errs.push(html_quote(error_formatter.format(&err).as_ref()));
                    }
                },
                async { download_media.execute(input).await }
            );
            let (media_for_upload, format) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(&*messenger, inline_message_id, &errs, Some(TextFormat::Html)).await;
                    return Ok(EventReturn::Finish);
                }
                Err(err) => {
                    error!(%err, "Download error");
                    let _ = progress::is_error_in_chosen_inline(
                        &*messenger,
                        inline_message_id,
                        &html_quote(error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            let _ = progress::is_sending_in_chosen_inline(&*messenger, inline_message_id).await;
            let file_id = match upload_media
                .execute(send_media::upload::SendVideoInput {
                    chat_id: cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_for_upload,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    width: format.width,
                    height: format.height,
                    #[allow(clippy::cast_possible_truncation)]
                    duration: media.duration.map(|val| val as i64),
                    with_delete: true,
                    webpage_url: &media.webpage_url,
                    link_is_visible: true,
                })
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = format_error_report(&err), "Send error");
                    let _ = progress::is_error_in_chosen_inline(
                        &*messenger,
                        inline_message_id,
                        &html_quote(error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
                return Ok(EventReturn::Finish);
            }

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
                error!(%err, "Add error");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let text = "Playlist is empty";
            let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, text, Some(TextFormat::Html)).await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get error");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
            );
            let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
            return Ok(EventReturn::Finish);
        }
        _ => unreachable!("Incorrect branch"),
    }
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, url, ?params))]
pub async fn download_audio<Messenger>(
    params: Params,
    url_option: Option<Extension<Url>>,
    Extension(chat_cfg): Extension<ChatConfig>,
    ChosenInlineResult {
        inline_message_id,
        result_id,
        ..
    }: ChosenInlineResult,
    Inject(cfg): Inject<Config>,
    Inject(error_formatter): Inject<ErrorMessageFormatter>,
    Inject(messenger): Inject<Messenger>,
    Inject(get_media): Inject<get_media::GetAudioByURL>,
    Inject(download_media): Inject<media::DownloadAudio>,
    Inject(upload_media): Inject<send_media::upload::SendAudio<Messenger>>,
    Inject(edit_media_by_id): Inject<send_media::id::EditAudio<Messenger>>,
    Inject(add_downloaded_media): Inject<downloaded_media::AddAudio>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let url = if let Some(Extension(val)) = url_option {
        val
    } else {
        let (_, video_id) = result_id.split_once('_').expect("Incorrect inline message ID");
        Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).unwrap()
    };

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("url", url.as_str());

    debug!("Got url");

    let playlist_range = Range::default();
    let sections = match params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(%err, "Parse sections error");
                let text = format!(
                    "Sorry, an error to parse sections\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
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
            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let mut download_errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (input, mut err_receiver, mut progress_receiver) =
                media::DownloadMediaInput::new_with_progress(&url, &media, sections.as_ref(), formats);

            let ((), (), download_res) = tokio::join!(
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if progress::is_downloading_with_progress_in_chosen_inline(&*messenger, inline_message_id, progress_str)
                            .await
                            .is_err()
                        {
                            break;
                        };
                    }
                },
                async {
                    while let Some(err) = err_receiver.recv().await {
                        download_errs.push(html_quote(error_formatter.format(&err).as_ref()));
                    }
                },
                async { download_media.execute(input).await }
            );
            let (media_for_upload, _format) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ =
                        progress::is_errors_in_chosen_inline(&*messenger, inline_message_id, &download_errs, Some(TextFormat::Html)).await;
                    return Ok(EventReturn::Finish);
                }
                Err(err) => {
                    error!(%err, "Download error");
                    let _ = progress::is_error_in_chosen_inline(
                        &*messenger,
                        inline_message_id,
                        &html_quote(error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            let _ = progress::is_sending_in_chosen_inline(&*messenger, inline_message_id).await;
            let file_id = match upload_media
                .execute(send_media::upload::SendAudioInput {
                    chat_id: cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_for_upload,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    title: media.title.as_deref(),
                    performer: media.uploader.as_deref(),
                    #[allow(clippy::cast_possible_truncation)]
                    duration: media.duration.map(|val| val as i64),
                    with_delete: true,
                    webpage_url: &media.webpage_url,
                    link_is_visible: true,
                })
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = format_error_report(&err), "Send error");
                    let _ = progress::is_error_in_chosen_inline(
                        &*messenger,
                        inline_message_id,
                        &html_quote(error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(EventReturn::Finish);
                }
            };

            if let Err(err) = edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
                return Ok(EventReturn::Finish);
            }

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
                error!(%err, "Add error");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let text = "Playlist is empty";
            let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, text, Some(TextFormat::Html)).await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get error");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(error_formatter.format(&err).as_ref()))
            );
            let _ = progress::is_error_in_chosen_inline(&*messenger, inline_message_id, &text, Some(TextFormat::Html)).await;
            return Ok(EventReturn::Finish);
        }
        _ => unreachable!("Incorrect branch"),
    }
    Ok(EventReturn::Finish)
}
