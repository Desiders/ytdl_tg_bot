use std::{borrow::Cow, str::FromStr as _, sync::Arc};

use rust_i18n::t;
use telers::{
    errors::HandlerError,
    utils::text::{html_expandable_blockquote, html_quote},
};
use tracing::{debug, error, instrument, warn};
use url::Url;

use crate::{
    config::{AudioFirstConfig, Config},
    entities::{language::Language, ChatConfig, Params, Range, Sections},
    handlers_utils::progress,
    interactors::{auto, Interactor},
    services::{
        download::media,
        downloaded_media,
        get_media::{
            self,
            GetMediaByURLKind::{self, Empty, Playlist, SingleCached},
        },
        messenger::{EditTarget, MessengerPort, TextFormat},
        send_media,
    },
    utils::{ErrorFormatter, FormatErrorToMessage},
    value_objects::MediaType,
};

pub struct DownloadVideo<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    get_media: Arc<get_media::GetVideoByURL>,
    download_media: Arc<media::DownloadVideo>,
    upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
    edit_media_by_id: Arc<send_media::id::EditVideo<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddVideo>,
}

impl<Messenger> DownloadVideo<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        get_media: Arc<get_media::GetVideoByURL>,
        download_media: Arc<media::DownloadVideo>,
        upload_media: Arc<send_media::upload::SendVideo<Messenger>>,
        edit_media_by_id: Arc<send_media::id::EditVideo<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddVideo>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
            get_media,
            download_media,
            upload_media,
            edit_media_by_id,
            add_downloaded_media,
        }
    }
}

pub struct DownloadAudio<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    get_media: Arc<get_media::GetAudioByURL>,
    download_media: Arc<media::DownloadAudio>,
    upload_media: Arc<send_media::upload::SendAudio<Messenger>>,
    edit_media_by_id: Arc<send_media::id::EditAudio<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddAudio>,
}

impl<Messenger> DownloadAudio<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        get_media: Arc<get_media::GetAudioByURL>,
        download_media: Arc<media::DownloadAudio>,
        upload_media: Arc<send_media::upload::SendAudio<Messenger>>,
        edit_media_by_id: Arc<send_media::id::EditAudio<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddAudio>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
            get_media,
            download_media,
            upload_media,
            edit_media_by_id,
            add_downloaded_media,
        }
    }
}

pub struct DownloadPhoto<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    get_media: Arc<get_media::GetPhotoByURL>,
    upload_media: Arc<send_media::upload::SendPhotoUrl<Messenger>>,
    edit_media_by_id: Arc<send_media::id::EditPhoto<Messenger>>,
    add_downloaded_media: Arc<downloaded_media::AddPhoto>,
}

impl<Messenger> DownloadPhoto<Messenger> {
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        get_media: Arc<get_media::GetPhotoByURL>,
        upload_media: Arc<send_media::upload::SendPhotoUrl<Messenger>>,
        edit_media_by_id: Arc<send_media::id::EditPhoto<Messenger>>,
        add_downloaded_media: Arc<downloaded_media::AddPhoto>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
            get_media,
            upload_media,
            edit_media_by_id,
            add_downloaded_media,
        }
    }
}

pub struct DownloadInput<'a> {
    pub guest: bool,
    pub params: &'a Params,
    pub url: Option<&'a Url>,
    pub chat_cfg: &'a ChatConfig,
    pub link_is_visible: bool,
    pub inline_message_id: &'a str,
    pub result_id: &'a str,
    pub prefetched: Option<GetMediaByURLKind>,
}

impl DownloadInput<'_> {
    fn format_error(&self, formatter: &ErrorFormatter, err: &(impl FormatErrorToMessage + ?Sized)) -> Cow<'static, str> {
        if self.guest {
            Cow::Owned(t!("guest.error", locale = self.chat_cfg.locale().as_str()).into_owned())
        } else {
            formatter.format(err)
        }
    }

    fn log_error(&self, formatter: &ErrorFormatter, err: &(impl FormatErrorToMessage + ?Sized)) -> Cow<'static, str> {
        if self.guest {
            Cow::Borrowed(err.category())
        } else {
            formatter.format(err)
        }
    }

    fn select_media(&self, media: GetMediaByURLKind) -> GetMediaByURLKind {
        if self.guest {
            // Photo metadata may contain a whole album even for Range::default().
            media.into_first()
        } else {
            media
        }
    }
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

impl<Messenger> Interactor<DownloadInput<'_>> for &DownloadPhoto<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(inline_message_id = input.inline_message_id, ?input.params))]
    async fn execute(self, input: DownloadInput<'_>) -> Result<Self::Output, Self::Err> {
        execute_photo(self, input).await
    }
}

async fn execute_video<Messenger>(interactor: &DownloadVideo<Messenger>, mut input: DownloadInput<'_>) -> Result<(), HandlerError>
where
    Messenger: MessengerPort,
{
    let url = resolve_url(input.url, input.result_id);
    debug!("Got url");
    let locale = input.chat_cfg.locale();

    let playlist_range = Range::default();
    let sections = match input.params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Parse sections error");
                let text = format!(
                    "{}\n{}",
                    t!("download.error_parse_sections", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
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
    let overwrite_cache = input.params.get_bool("overwrite");

    let result = match input.prefetched.take() {
        Some(result) => Ok(result),
        None => {
            interactor
                .get_media
                .execute(get_media::GetMediaByURLInput {
                    url: &url,
                    playlist_range: &playlist_range,
                    cache_search: url.as_str(),
                    domain: url.domain(),
                    audio_language: &audio_language,
                    sections: sections.as_ref(),
                    overwrite_cache,
                })
                .await
        }
    };
    match result.map(|media| input.select_media(media)) {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let mut errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (download_input, mut err_receiver, mut progress_receiver) =
                media::DownloadMediaInput::new_with_progress(&url, &media, sections.as_ref(), formats);

            let download_res = progress::with_optional_updates(interactor.download_media.execute(download_input), async {
                while let Some(progress_str) = progress_receiver.recv().await {
                    if input.guest {
                        continue;
                    }
                    if progress::is_downloading_with_progress_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        progress_str,
                        input.chat_cfg.locale().as_str(),
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
            })
            .await;
            while let Some(err) = err_receiver.recv().await {
                if input.guest {
                    warn!(category = err.category(), "Guest download candidate failed");
                }
                errs.push(html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()));
            }

            let (media_for_upload, format, duration) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &errs,
                        Some(TextFormat::Html),
                        input.chat_cfg.locale().as_str(),
                    )
                    .await;
                    return Ok(());
                }
                Err(err) => {
                    error!(err = %input.log_error(&interactor.error_formatter, &err), "Download error");
                    let _ = progress::is_error_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            let file_id = match progress::while_sending(
                interactor.upload_media.execute(send_media::upload::SendVideoInput {
                    chat_id: interactor.cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_for_upload,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    width: format.width,
                    height: format.height,
                    duration,
                    with_delete: true,
                    webpage_url: &media.webpage_url,
                    link_is_visible: !input.guest,
                }),
                interactor.messenger.as_ref(),
                EditTarget::InlineMessage {
                    inline_message_id: input.inline_message_id,
                },
                input.chat_cfg.locale().as_str(),
                None,
            )
            .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = %input.log_error(&interactor.error_formatter, &err), "Send error");
                    let err = input.format_error(&interactor.error_formatter, &err);
                    let _ = progress::is_error_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &progress::upload_error(err.as_ref(), input.chat_cfg.locale().as_str()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
                return Ok(());
            }

            if let Err(err) = interactor
                .add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id,
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: audio_language.clone(),
                    sections: sections.clone(),
                    overwrite_cache,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Add error");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let _ = progress::is_error_in_chosen_inline(
                interactor.messenger.as_ref(),
                input.inline_message_id,
                t!("download.playlist_empty", locale = locale.as_str()).as_ref(),
                Some(TextFormat::Html),
            )
            .await;
        }
        Err(err) => {
            error!(err = %input.log_error(&interactor.error_formatter, &err), "Get error");
            let text = format!(
                "{}\n{}",
                t!("download.error_get_info", locale = locale.as_str()),
                html_expandable_blockquote(html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()))
            );
            let _ = progress::is_error_in_chosen_inline(
                interactor.messenger.as_ref(),
                input.inline_message_id,
                &text,
                Some(TextFormat::Html),
            )
            .await;
        }
        _ => unreachable!("Incorrect branch"),
    }

    Ok(())
}

async fn execute_audio<Messenger>(interactor: &DownloadAudio<Messenger>, mut input: DownloadInput<'_>) -> Result<(), HandlerError>
where
    Messenger: MessengerPort,
{
    let url = resolve_url(input.url, input.result_id);
    debug!("Got url");
    let locale = input.chat_cfg.locale();

    let playlist_range = Range::default();
    let sections = match input.params.0.get("crop") {
        Some(raw_value) => Some(match Sections::from_str(raw_value) {
            Ok(val) => val,
            Err(err) => {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Parse sections error");
                let text = format!(
                    "{}\n{}",
                    t!("download.error_parse_sections", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
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
    let overwrite_cache = input.params.get_bool("overwrite");

    let result = match input.prefetched.take() {
        Some(result) => Ok(result),
        None => {
            interactor
                .get_media
                .execute(get_media::GetMediaByURLInput {
                    url: &url,
                    playlist_range: &playlist_range,
                    cache_search: url.as_str(),
                    domain: url.domain(),
                    audio_language: &audio_language,
                    sections: sections.as_ref(),
                    overwrite_cache,
                })
                .await
        }
    };
    match result.map(|media| input.select_media(media)) {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            let file_id = media.file_id;

            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let mut download_errs = vec![];
            let (media, formats) = uncached.remove(0);

            let (download_input, mut err_receiver, mut progress_receiver) =
                media::DownloadMediaInput::new_with_progress(&url, &media, sections.as_ref(), formats);

            let download_res = progress::with_optional_updates(interactor.download_media.execute(download_input), async {
                while let Some(progress_str) = progress_receiver.recv().await {
                    if input.guest {
                        continue;
                    }
                    if progress::is_downloading_with_progress_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        progress_str,
                        input.chat_cfg.locale().as_str(),
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                }
            })
            .await;
            while let Some(err) = err_receiver.recv().await {
                if input.guest {
                    warn!(category = err.category(), "Guest download candidate failed");
                }
                download_errs.push(html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()));
            }

            let (media_for_upload, _format, duration) = match download_res {
                Ok(Some(val)) => val,
                Ok(None) => {
                    let _ = progress::is_errors_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &download_errs,
                        Some(TextFormat::Html),
                        input.chat_cfg.locale().as_str(),
                    )
                    .await;
                    return Ok(());
                }
                Err(err) => {
                    error!(err = %input.log_error(&interactor.error_formatter, &err), "Download error");
                    let _ = progress::is_error_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            let file_id = match progress::while_sending(
                interactor.upload_media.execute(send_media::upload::SendAudioInput {
                    chat_id: interactor.cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    media_for_upload,
                    name: media.title.as_deref().unwrap_or(media.id.as_ref()),
                    title: media.title.as_deref(),
                    performer: media.uploader.as_deref(),
                    duration,
                    with_delete: true,
                    webpage_url: &media.webpage_url,
                    link_is_visible: !input.guest,
                }),
                interactor.messenger.as_ref(),
                EditTarget::InlineMessage {
                    inline_message_id: input.inline_message_id,
                },
                input.chat_cfg.locale().as_str(),
                None,
            )
            .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = %input.log_error(&interactor.error_formatter, &err), "Send error");
                    let err = input.format_error(&interactor.error_formatter, &err);
                    let _ = progress::is_error_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &progress::upload_error(err.as_ref(), input.chat_cfg.locale().as_str()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
                return Ok(());
            }

            if let Err(err) = interactor
                .add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id,
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: audio_language.clone(),
                    sections: sections.clone(),
                    overwrite_cache,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Add error");
            }
        }
        Ok(Empty) => {
            warn!("Empty playlist");
            let _ = progress::is_error_in_chosen_inline(
                interactor.messenger.as_ref(),
                input.inline_message_id,
                t!("download.playlist_empty", locale = locale.as_str()).as_ref(),
                Some(TextFormat::Html),
            )
            .await;
        }
        Err(err) => {
            error!(err = %input.log_error(&interactor.error_formatter, &err), "Get error");
            let text = format!(
                "{}\n{}",
                t!("download.error_get_info", locale = locale.as_str()),
                html_expandable_blockquote(html_quote(input.format_error(&interactor.error_formatter, &err).as_ref()))
            );
            let _ = progress::is_error_in_chosen_inline(
                interactor.messenger.as_ref(),
                input.inline_message_id,
                &text,
                Some(TextFormat::Html),
            )
            .await;
        }
        _ => unreachable!("Incorrect branch"),
    }

    Ok(())
}

async fn execute_photo<Messenger>(interactor: &DownloadPhoto<Messenger>, mut input: DownloadInput<'_>) -> Result<(), HandlerError>
where
    Messenger: MessengerPort,
{
    let url = resolve_url(input.url, input.result_id);
    debug!("Got url");
    let locale = input.chat_cfg.locale();

    let playlist_range = Range::default();
    let overwrite_cache = input.params.get_bool("overwrite");

    let result = match input.prefetched.take() {
        Some(result) => Ok(result),
        None => {
            interactor
                .get_media
                .execute(get_media::GetMediaByURLInput {
                    url: &url,
                    playlist_range: &playlist_range,
                    cache_search: url.as_str(),
                    domain: url.domain(),
                    audio_language: &Language::default(),
                    sections: None,
                    overwrite_cache,
                })
                .await
        }
    };
    match result.map(|media| input.select_media(media)) {
        Ok(SingleCached(file_id)) => {
            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&url),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }
        Ok(Playlist { mut cached, .. }) if !cached.is_empty() => {
            let media = cached.remove(0);
            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &media.file_id,
                    webpage_url: media.webpage_url.as_ref(),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
            }
        }
        Ok(Playlist { mut uncached, .. }) if !uncached.is_empty() => {
            let (media, _formats) = uncached.remove(0);
            let Some(photo_url) = media.direct_url.as_ref() else {
                error!("Photo URL is missing in downloader response");
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &html_quote("Photo URL is missing in downloader response"),
                    Some(TextFormat::Html),
                )
                .await;
                return Ok(());
            };

            let file_id = match interactor
                .upload_media
                .execute(send_media::upload::SendPhotoUrlInput {
                    chat_id: interactor.cfg.chat.receiver_chat_id,
                    reply_to_message_id: None,
                    photo_url,
                    with_delete: true,
                    webpage_url: &media.webpage_url,
                    link_is_visible: !input.guest,
                })
                .await
            {
                Ok(val) => val,
                Err(err) => {
                    error!(err = %input.log_error(&interactor.error_formatter, &err), "Send error");
                    let err = input.format_error(&interactor.error_formatter, &err);
                    let _ = progress::is_error_in_chosen_inline(
                        interactor.messenger.as_ref(),
                        input.inline_message_id,
                        &progress::upload_error(err.as_ref(), input.chat_cfg.locale().as_str()),
                        Some(TextFormat::Html),
                    )
                    .await;
                    return Ok(());
                }
            };

            if let Err(err) = interactor
                .edit_media_by_id
                .execute(send_media::id::EditMediaInput {
                    inline_message_id: input.inline_message_id,
                    id: &file_id,
                    webpage_url: Some(&media.webpage_url),
                    link_is_visible: input.link_is_visible,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Edit error");
                let err = input.format_error(&interactor.error_formatter, &err);
                let text = format!(
                    "{}\n{}",
                    t!("download.error_edit_message", locale = locale.as_str()),
                    html_expandable_blockquote(html_quote(err.as_ref()))
                );
                let _ = progress::is_error_in_chosen_inline(
                    interactor.messenger.as_ref(),
                    input.inline_message_id,
                    &text,
                    Some(TextFormat::Html),
                )
                .await;
                return Ok(());
            }

            if let Err(err) = interactor
                .add_downloaded_media
                .execute(downloaded_media::AddMediaInput {
                    file_id,
                    id: media.id.clone(),
                    display_id: media.display_id.clone(),
                    domain: media.webpage_url.host_str().map(ToOwned::to_owned),
                    audio_language: Language::default(),
                    sections: None,
                    overwrite_cache,
                })
                .await
            {
                error!(err = %input.log_error(&interactor.error_formatter, &err), "Add error");
            }
        }
        Ok(Empty) => {
            warn!("No media");
            let _ = progress::is_error_in_chosen_inline(
                interactor.messenger.as_ref(),
                input.inline_message_id,
                t!("download.no_media_found", locale = locale.as_str()).as_ref(),
                None,
            )
            .await;
        }
        Err(err) => {
            let formatted = input.format_error(&interactor.error_formatter, &err);
            error!(err = %input.log_error(&interactor.error_formatter, &err), "Get error");
            let text = format!(
                "{}\n{}",
                t!("download.error_get_media", locale = locale.as_str()),
                html_expandable_blockquote(html_quote(formatted.as_ref()))
            );
            let _ = progress::is_error_in_chosen_inline(
                interactor.messenger.as_ref(),
                input.inline_message_id,
                &text,
                Some(TextFormat::Html),
            )
            .await;
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

// Inline auto: classify the link (video -> audio -> photo), then run that type's inline downloader.
pub struct DownloadAuto<Messenger> {
    audio_first: Arc<AudioFirstConfig>,
    get_video: Arc<get_media::GetVideoByURL>,
    get_audio: Arc<get_media::GetAudioByURL>,
    get_photo: Arc<get_media::GetPhotoByURL>,
    video: Arc<DownloadVideo<Messenger>>,
    audio: Arc<DownloadAudio<Messenger>>,
    photo: Arc<DownloadPhoto<Messenger>>,
}

impl<Messenger> DownloadAuto<Messenger> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        audio_first: Arc<AudioFirstConfig>,
        get_video: Arc<get_media::GetVideoByURL>,
        get_audio: Arc<get_media::GetAudioByURL>,
        get_photo: Arc<get_media::GetPhotoByURL>,
        video: Arc<DownloadVideo<Messenger>>,
        audio: Arc<DownloadAudio<Messenger>>,
        photo: Arc<DownloadPhoto<Messenger>>,
    ) -> Self {
        Self {
            audio_first,
            get_video,
            get_audio,
            get_photo,
            video,
            audio,
            photo,
        }
    }
}

impl<Messenger> Interactor<DownloadInput<'_>> for &DownloadAuto<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(inline_message_id = input.inline_message_id, result_id = input.result_id))]
    async fn execute(self, input: DownloadInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Got inline auto");

        let url = resolve_url(input.url, input.result_id);
        let playlist_range = if input.guest {
            Range::default()
        } else {
            input
                .params
                .0
                .get("items")
                .and_then(|raw| Range::from_str(raw).ok())
                .unwrap_or_default()
        };
        let sections = input.params.0.get("crop").and_then(|raw| Sections::from_str(raw).ok());
        let audio_language = input
            .params
            .0
            .get("lang")
            .and_then(|raw| Language::from_str(raw).ok())
            .unwrap_or_default();
        let overwrite_cache = input.params.get_bool("overwrite");

        let (media_type, prefetched) = auto::classify(
            self.get_video.as_ref(),
            self.get_audio.as_ref(),
            self.get_photo.as_ref(),
            &url,
            &playlist_range,
            sections.as_ref(),
            &audio_language,
            overwrite_cache,
            auto::is_audio_first(&self.audio_first, &url),
        )
        .await;

        let input = DownloadInput { prefetched, ..input };
        match media_type {
            MediaType::Video => self.video.execute(input).await,
            MediaType::Audio => self.audio.execute(input).await,
            MediaType::Photo => self.photo.execute(input).await,
        }
    }
}

#[cfg(test)]
mod guest_tests {
    use super::*;
    use crate::services::messenger::MessengerError;

    #[test]
    fn guest_diagnostics_keep_categories_without_exposing_error_payloads() {
        let params = Params::default();
        let chat = ChatConfig::new(0, false, "en".into());
        let input = DownloadInput {
            guest: true,
            params: &params,
            url: None,
            chat_cfg: &chat,
            link_is_visible: false,
            inline_message_id: "synthetic-inline",
            result_id: "guest",
            prefetched: None,
        };
        let formatter = ErrorFormatter::new("synthetic-token");
        let error = MessengerError::with_category(
            "Synthetic private URL https://example.test/private?secret=synthetic-token",
            "telegram_forbidden",
        );
        assert_eq!(input.log_error(&formatter, &error), "telegram_forbidden");
        let reply = input.format_error(&formatter, &error);
        assert!(!reply.contains("telegram_forbidden"));
        assert!(!reply.contains("secret"));
        assert!(!reply.contains("example.test"));
        let error = crate::services::node_router::DownloadErrorKind::MediaTimeout;
        assert_eq!(input.log_error(&formatter, &error), "media_timeout");
        let error = crate::services::node_router::DownloadErrorKind::ExecutionUncertain;
        assert_eq!(input.log_error(&formatter, &error), "execution_uncertain");
    }
}
