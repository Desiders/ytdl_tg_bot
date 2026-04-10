use std::{str::FromStr as _, sync::Arc};

use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::{debug, error, instrument, warn};
use url::Url;

use crate::{
    config::Config,
    database::TxManager,
    entities::{language::Language, ChatConfig, Params, Range, Sections},
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        download::media,
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{Empty, Playlist, SingleCached},
        },
        messenger::{MessengerPort, TextFormat},
        send_media,
    },
    utils::{format_error_report, ErrorMessageFormatter},
};

pub struct DownloadVideo<Messenger> {
    pub cfg: Arc<Config>,
    pub error_formatter: Arc<ErrorMessageFormatter>,
    pub messenger: Arc<Messenger>,
    pub get_media: Arc<get_media::GetVideoByURL>,
    pub download_media: Arc<media::DownloadVideo>,
    pub upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
    pub edit_media_by_id: Arc<send_media::id::EditVideo<Messenger>>,
    pub add_downloaded_media: Arc<downloaded_media::AddVideo>,
}

pub struct DownloadAudio<Messenger> {
    pub cfg: Arc<Config>,
    pub error_formatter: Arc<ErrorMessageFormatter>,
    pub messenger: Arc<Messenger>,
    pub get_media: Arc<get_media::GetAudioByURL>,
    pub download_media: Arc<media::DownloadAudio>,
    pub upload_media: Arc<send_media::upload::SendAudio<Messenger>>,
    pub edit_media_by_id: Arc<send_media::id::EditAudio<Messenger>>,
    pub add_downloaded_media: Arc<downloaded_media::AddAudio>,
}

pub struct DownloadInput<'a> {
    pub params: &'a Params,
    pub url: Option<&'a Url>,
    pub chat_cfg: &'a ChatConfig,
    pub inline_message_id: &'a str,
    pub result_id: &'a str,
    pub tx_manager: &'a mut TxManager,
}

impl<Messenger> Interactor<DownloadInput<'_>> for &DownloadVideo<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(inline_message_id = input.inline_message_id, ?input.params))]
    async fn execute(self, input: DownloadInput<'_>) -> Result<Self::Output, Self::Err> {
        execute_video(self, input).await
    }
}

impl<Messenger> Interactor<DownloadInput<'_>> for &DownloadAudio<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(inline_message_id = input.inline_message_id, ?input.params))]
    async fn execute(self, input: DownloadInput<'_>) -> Result<Self::Output, Self::Err> {
        execute_audio(self, input).await
    }
}

async fn execute_video<Messenger>(this: &DownloadVideo<Messenger>, input: DownloadInput<'_>) -> Result<(), HandlerError>
where
    Messenger: MessengerPort,
{
    let url = resolve_url(input.url, input.result_id);
    debug!("Got url");

    let playlist_range = Range::default();
    let sections = match input.params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(%err, "Parse sections error");
                let text = format!(
                    "Sorry, an error to parse sections\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
                return Ok(());
            }
        }),
        None => None,
    };
    let audio_language = match input.params.0.get("lang") {
        Some(raw_value) => Language::from_str(raw_value).unwrap(),
        None => Language::default(),
    };

    match this
        .get_media
        .execute(get_media::GetMediaByURLInput {
            url: &url,
            playlist_range: &playlist_range,
            cache_search: url.as_str(),
            domain: url.domain(),
            audio_language: &audio_language,
            sections: sections.as_ref(),
            tx_manager: input.tx_manager,
        })
        .await
    {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = this
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: input.chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = this
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: input.chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let mut errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (download_input, mut err_receiver, mut progress_receiver) =
                media::DownloadMediaInput::new_with_progress(&url, &media, sections.as_ref(), formats);

            let ((), (), download_res) = tokio::join!(
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if progress::is_downloading_with_progress_in_chosen_inline(
                            this.messenger.as_ref(),
                            input.inline_message_id,
                            progress_str,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                },
                async {
                    while let Some(err) = err_receiver.recv().await {
                        errs.push(html_quote(this.error_formatter.format(&err).as_ref()));
                    }
                },
                async { this.download_media.execute(download_input).await }
            );

            let (media_for_upload, format) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(
                        this.messenger.as_ref(),
                        input.inline_message_id,
                        &errs,
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
                Err(err) => {
                    error!(%err, "Download error");
                    let _ = progress::is_error_in_chosen_inline(
                        this.messenger.as_ref(),
                        input.inline_message_id,
                        &html_quote(this.error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            let _ = progress::is_sending_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id).await;
            let file_id = match this
                .upload_media
                .execute(send_media::upload::SendVideoInput {
                    chat_id: this.cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_for_upload,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    width: format.width,
                    height: format.height,
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
                        this.messenger.as_ref(),
                        input.inline_message_id,
                        &html_quote(this.error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            if let Err(err) = this
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: input.chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
                return Ok(());
            }

            if let Err(err) = this
                .add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id: file_id.into(),
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: audio_language.clone(),
                    sections: sections.clone(),
                    tx_manager: input.tx_manager,
                })
                .await
            {
                error!(%err, "Add error");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let _ = progress::is_error_in_chosen_inline(
                this.messenger.as_ref(),
                input.inline_message_id,
                "Playlist is empty",
                Some(TextFormat::Html),
            )
            .await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get error");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
            );
            let _ =
                progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html)).await;
        }
        _ => unreachable!("Incorrect branch"),
    }

    Ok(())
}

async fn execute_audio<Messenger>(this: &DownloadAudio<Messenger>, input: DownloadInput<'_>) -> Result<(), HandlerError>
where
    Messenger: MessengerPort,
{
    let url = resolve_url(input.url, input.result_id);
    debug!("Got url");

    let playlist_range = Range::default();
    let sections = match input.params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(%err, "Parse sections error");
                let text = format!(
                    "Sorry, an error to parse sections\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
                return Ok(());
            }
        }),
        None => None,
    };
    let audio_language = match input.params.0.get("lang") {
        Some(raw_value) => Language::from_str(raw_value).unwrap(),
        None => Language::default(),
    };

    match this
        .get_media
        .execute(get_media::GetMediaByURLInput {
            url: &url,
            playlist_range: &playlist_range,
            cache_search: url.as_str(),
            domain: url.domain(),
            audio_language: &audio_language,
            sections: sections.as_ref(),
            tx_manager: input.tx_manager,
        })
        .await
    {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = this
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: input.chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = this
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: input.chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let mut download_errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (download_input, mut err_receiver, mut progress_receiver) =
                media::DownloadMediaInput::new_with_progress(&url, &media, sections.as_ref(), formats);

            let ((), (), download_res) = tokio::join!(
                async {
                    while let Some(progress_str) = progress_receiver.recv().await {
                        if progress::is_downloading_with_progress_in_chosen_inline(
                            this.messenger.as_ref(),
                            input.inline_message_id,
                            progress_str,
                        )
                        .await
                        .is_err()
                        {
                            break;
                        }
                    }
                },
                async {
                    while let Some(err) = err_receiver.recv().await {
                        download_errs.push(html_quote(this.error_formatter.format(&err).as_ref()));
                    }
                },
                async { this.download_media.execute(download_input).await }
            );

            let (media_for_upload, _format) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(
                        this.messenger.as_ref(),
                        input.inline_message_id,
                        &download_errs,
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
                Err(err) => {
                    error!(%err, "Download error");
                    let _ = progress::is_error_in_chosen_inline(
                        this.messenger.as_ref(),
                        input.inline_message_id,
                        &html_quote(this.error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            let _ = progress::is_sending_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id).await;
            let file_id = match this
                .upload_media
                .execute(send_media::upload::SendAudioInput {
                    chat_id: this.cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_for_upload,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    title: media.title.as_deref(),
                    performer: media.uploader.as_deref(),
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
                        this.messenger.as_ref(),
                        input.inline_message_id,
                        &html_quote(this.error_formatter.format(&err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            if let Err(err) = this
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: input.chat_cfg.link_is_visible,
                })
                .await
            {
                error!(err = format_error_report(&err), "Edit error");
                let text = format!(
                    "Sorry, an error to edit the message\n{}",
                    html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
                );
                let _ =
                    progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html))
                        .await;
                return Ok(());
            }

            if let Err(err) = this
                .add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id: file_id.into(),
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: audio_language.clone(),
                    sections: sections.clone(),
                    tx_manager: input.tx_manager,
                })
                .await
            {
                error!(%err, "Add error");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let _ = progress::is_error_in_chosen_inline(
                this.messenger.as_ref(),
                input.inline_message_id,
                "Playlist is empty",
                Some(TextFormat::Html),
            )
            .await;
        }
        Err(err) => {
            error!(err = format_error_report(&err), "Get error");
            let text = format!(
                "Sorry, an error to get info\n{}",
                html_expandable_blockquote(html_quote(this.error_formatter.format(&err).as_ref()))
            );
            let _ =
                progress::is_error_in_chosen_inline(this.messenger.as_ref(), input.inline_message_id, &text, Some(TextFormat::Html)).await;
        }
        _ => unreachable!("Incorrect branch"),
    }

    Ok(())
}

fn resolve_url(url: Option<&Url>, result_id: &str) -> Url {
    if let Some(url) = url {
        return url.clone();
    }

    let (_, video_id) = result_id.split_once('_').expect("Incorrect inline message ID");
    Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).expect("Invalid inline YouTube URL")
}
