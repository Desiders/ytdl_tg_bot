use crate::{
    config::{ChatConfig, YtDlpConfig},
    database::TxManager,
    entities::{AudioAndFormat, PreferredLanguages, Range, UrlWithParams, VideoAndFormat},
    handlers_utils::error,
    interactors::{
        download::{DownloadAudio, DownloadAudioInput, DownloadVideo, DownloadVideoInput},
        send_media::{
            EditAudioById, EditAudioByIdInput, EditVideoById, EditVideoByIdInput, SendAudioInFS, SendAudioInFSInput, SendVideoInFS,
            SendVideoInFSInput,
        },
        AddDownloadedAudio, AddDownloadedMediaInput, AddDownloadedVideo, GetAudioByURL, GetAudioByURLInput, GetAudioByURLKind,
        GetVideoByURL, GetVideoByURLInput, GetVideoByURLKind, Interactor as _,
    },
    utils::{format_error_report, FormatErrorToMessage as _},
};

use froodi::{Inject, InjectTransient};
use std::str::FromStr as _;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    types::ChosenInlineResult,
    utils::text::{html_formatter::expandable_blockquote, html_text_link},
    Bot, Extension,
};
use tracing::{event, instrument, Level, Span};
use url::Url;

#[instrument(skip_all, fields(inline_message_id, is_video, url = url.as_str(), ?params))]
pub async fn download_by_url(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        from,
        ..
    }: ChosenInlineResult,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_video_by_url): Inject<GetVideoByURL>,
    Inject(get_audio_by_url): Inject<GetAudioByURL>,
    Inject(download_video): Inject<DownloadVideo>,
    Inject(download_audio): Inject<DownloadAudio>,
    Inject(send_video_in_fs): Inject<SendVideoInFS>,
    Inject(edit_video_by_id): Inject<EditVideoById>,
    Inject(send_audio_in_fs): Inject<SendAudioInFS>,
    Inject(edit_audio_by_id): Inject<EditAudioById>,
    Inject(add_downloaded_video): Inject<AddDownloadedVideo>,
    Inject(add_downloaded_audio): Inject<AddDownloadedAudio>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let is_video = result_id.starts_with("video_");
    let chat_id = from.id;

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("is_video", is_video);

    event!(Level::DEBUG, "Got url");

    let preferred_languages = match params.get("lang") {
        Some(raw_value) => PreferredLanguages::from_str(raw_value).unwrap(),
        None => PreferredLanguages::default(),
    };
    if is_video {
        let file_id = match get_video_by_url
            .execute(GetVideoByURLInput::new(
                &url,
                &Range::default(),
                url.as_str(),
                url.domain().as_deref(),
                &mut tx_manager,
            ))
            .await
        {
            Ok(GetVideoByURLKind::SingleCached(file_id)) => file_id.into_boxed_str(),
            Ok(GetVideoByURLKind::Playlist((mut cached, mut uncached))) => {
                if cached.len() == 1 {
                    let cached = cached.remove(0);
                    cached.file_id
                } else {
                    let media = uncached.remove(0);
                    let media_and_format =
                        match VideoAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages) {
                            Ok(val) => val,
                            Err(err) => {
                                event!(Level::ERROR, %err, "Select format err");
                                let text = format!(
                                    "Sorry, an error to select a format\n{}",
                                    expandable_blockquote(err.format(&bot.token))
                                );
                                error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                                return Ok(EventReturn::Finish);
                            }
                        };
                    let media_in_fs = match download_video.execute(DownloadVideoInput::new(&url, media_and_format)).await {
                        Ok(val) => val,
                        Err(err) => {
                            event!(Level::ERROR, err = format_error_report(&err), "Download err");
                            let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                            return Ok(EventReturn::Finish);
                        }
                    };
                    let file_id = match send_video_in_fs
                        .execute(SendVideoInFSInput::new(
                            chat_cfg.receiver_chat_id,
                            None,
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
                        Ok(file_id) => file_id,
                        Err(err) => {
                            event!(Level::ERROR, err = format_error_report(&err), "Send err");
                            let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                            return Ok(EventReturn::Finish);
                        }
                    };

                    if let Err(err) = add_downloaded_video
                        .execute(AddDownloadedMediaInput::new(
                            file_id.clone().into(),
                            media.id.clone(),
                            media.display_id.clone(),
                            media.domain(),
                            chat_id,
                            &mut tx_manager,
                        ))
                        .await
                    {
                        event!(Level::ERROR, %err, "Add err");
                    }
                    file_id
                }
            }
            Ok(GetVideoByURLKind::Empty) => {
                event!(Level::ERROR, "Video not found");
                error::occured_in_chosen_inline_result(&bot, "Video not found", inline_message_id, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Get info err");
                let text = format!(
                    "Sorry, an error to get media info\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        if let Err(err) = edit_video_by_id
            .execute(EditVideoByIdInput::new(inline_message_id, &file_id, &html_text_link("Link", url)))
            .await
        {
            event!(Level::ERROR, err = format_error_report(&err), "Edit err");
            let text = format!(
                "Sorry, an error to edit the message\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
        }
        return Ok(EventReturn::Finish);
    }

    let file_id = match get_audio_by_url
        .execute(GetAudioByURLInput::new(
            &url,
            &Range::default(),
            url.as_str(),
            url.domain().as_deref(),
            &mut tx_manager,
        ))
        .await
    {
        Ok(GetAudioByURLKind::SingleCached(file_id)) => file_id.into_boxed_str(),
        Ok(GetAudioByURLKind::Playlist((mut cached, mut uncached))) => {
            if cached.len() == 1 {
                let cached = cached.remove(0);
                cached.file_id
            } else {
                let media = uncached.remove(0);
                let media_and_format = match AudioAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages)
                {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, %err, "Select format err");
                        let text = format!(
                            "Sorry, an error to select a format\n{}",
                            expandable_blockquote(err.format(&bot.token))
                        );
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                let media_in_fs = match download_audio.execute(DownloadAudioInput::new(&url, media_and_format)).await {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Download err");
                        let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                let file_id = match send_audio_in_fs
                    .execute(SendAudioInFSInput::new(
                        chat_cfg.receiver_chat_id,
                        None,
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
                    Ok(file_id) => file_id,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Send err");
                        let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                if let Err(err) = add_downloaded_audio
                    .execute(AddDownloadedMediaInput::new(
                        file_id.clone().into(),
                        media.id.clone(),
                        media.display_id.clone(),
                        media.domain(),
                        chat_id,
                        &mut tx_manager,
                    ))
                    .await
                {
                    event!(Level::ERROR, %err, "Add err");
                }
                file_id
            }
        }
        Ok(GetAudioByURLKind::Empty) => {
            event!(Level::ERROR, "Audio not found");
            error::occured_in_chosen_inline_result(&bot, "Audio not found", inline_message_id, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Get info err");
            let text = format!(
                "Sorry, an error to get media info\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
    };
    if let Err(err) = edit_audio_by_id
        .execute(EditAudioByIdInput::new(inline_message_id, &file_id, &html_text_link("Link", url)))
        .await
    {
        event!(Level::ERROR, err = format_error_report(&err), "Edit err");
        let text = format!(
            "Sorry, an error to edit the message\n{}",
            expandable_blockquote(err.format(&bot.token))
        );
        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
    }
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(inline_message_id, is_video, video_id))]
pub async fn download_by_id(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        from,
        ..
    }: ChosenInlineResult,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_video_by_url): Inject<GetVideoByURL>,
    Inject(get_audio_by_url): Inject<GetAudioByURL>,
    Inject(download_video): Inject<DownloadVideo>,
    Inject(download_audio): Inject<DownloadAudio>,
    Inject(send_video_in_fs): Inject<SendVideoInFS>,
    Inject(edit_video_by_id): Inject<EditVideoById>,
    Inject(send_audio_in_fs): Inject<SendAudioInFS>,
    Inject(edit_audio_by_id): Inject<EditAudioById>,
    Inject(add_downloaded_video): Inject<AddDownloadedVideo>,
    Inject(add_downloaded_audio): Inject<AddDownloadedAudio>,
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let (result_prefix, video_id) = result_id.split_once('_').unwrap();
    let url = Url::parse(&format!("https://www.youtube.com/watch?v={video_id}")).unwrap();
    let is_video = result_prefix.starts_with("video");
    let chat_id = from.id;

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("is_video", is_video)
        .record("video_id", video_id);

    event!(Level::DEBUG, "Got url");

    let preferred_languages = PreferredLanguages::default();
    if is_video {
        let file_id = match get_video_by_url
            .execute(GetVideoByURLInput::new(
                &url,
                &Range::default(),
                url.as_str(),
                url.domain(),
                &mut tx_manager,
            ))
            .await
        {
            Ok(GetVideoByURLKind::SingleCached(file_id)) => file_id.into_boxed_str(),
            Ok(GetVideoByURLKind::Playlist((mut cached, mut uncached))) => {
                if cached.len() == 1 {
                    let cached = cached.remove(0);
                    cached.file_id
                } else {
                    let media = uncached.remove(0);
                    let media_and_format =
                        match VideoAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages) {
                            Ok(val) => val,
                            Err(err) => {
                                event!(Level::ERROR, %err, "Select format err");
                                let text = format!(
                                    "Sorry, an error to select a format\n{}",
                                    expandable_blockquote(err.format(&bot.token))
                                );
                                error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                                return Ok(EventReturn::Finish);
                            }
                        };
                    let media_in_fs = match download_video.execute(DownloadVideoInput::new(&url, media_and_format)).await {
                        Ok(val) => val,
                        Err(err) => {
                            event!(Level::ERROR, err = format_error_report(&err), "Download err");
                            let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                            return Ok(EventReturn::Finish);
                        }
                    };
                    let file_id = match send_video_in_fs
                        .execute(SendVideoInFSInput::new(
                            chat_cfg.receiver_chat_id,
                            None,
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
                        Ok(file_id) => file_id,
                        Err(err) => {
                            event!(Level::ERROR, err = format_error_report(&err), "Send err");
                            let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                            return Ok(EventReturn::Finish);
                        }
                    };

                    if let Err(err) = add_downloaded_video
                        .execute(AddDownloadedMediaInput::new(
                            file_id.clone().into(),
                            media.id.clone(),
                            media.display_id.clone(),
                            media.domain(),
                            chat_id,
                            &mut tx_manager,
                        ))
                        .await
                    {
                        event!(Level::ERROR, %err, "Add err");
                    }
                    file_id
                }
            }
            Ok(GetVideoByURLKind::Empty) => {
                event!(Level::ERROR, "Video not found");
                error::occured_in_chosen_inline_result(&bot, "Video not found", inline_message_id, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Get info err");
                let text = format!(
                    "Sorry, an error to get media info\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        if let Err(err) = edit_video_by_id
            .execute(EditVideoByIdInput::new(inline_message_id, &file_id, &html_text_link("Link", url)))
            .await
        {
            event!(Level::ERROR, err = format_error_report(&err), "Edit err");
            let text = format!(
                "Sorry, an error to edit the message\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
        }
        return Ok(EventReturn::Finish);
    }

    let file_id = match get_audio_by_url
        .execute(GetAudioByURLInput::new(
            &url,
            &Range::default(),
            url.as_str(),
            url.domain().as_deref(),
            &mut tx_manager,
        ))
        .await
    {
        Ok(GetAudioByURLKind::SingleCached(file_id)) => file_id.into_boxed_str(),
        Ok(GetAudioByURLKind::Playlist((mut cached, mut uncached))) => {
            if cached.len() == 1 {
                let cached = cached.remove(0);
                cached.file_id
            } else {
                let media = uncached.remove(0);
                let media_and_format = match AudioAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages)
                {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, %err, "Select format err");
                        let text = format!(
                            "Sorry, an error to select a format\n{}",
                            expandable_blockquote(err.format(&bot.token))
                        );
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                let media_in_fs = match download_audio.execute(DownloadAudioInput::new(&url, media_and_format)).await {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Download err");
                        let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                let file_id = match send_audio_in_fs
                    .execute(SendAudioInFSInput::new(
                        chat_cfg.receiver_chat_id,
                        None,
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
                    Ok(file_id) => file_id,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Send err");
                        let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                if let Err(err) = add_downloaded_audio
                    .execute(AddDownloadedMediaInput::new(
                        file_id.clone().into(),
                        media.id.clone(),
                        media.display_id.clone(),
                        media.domain(),
                        chat_id,
                        &mut tx_manager,
                    ))
                    .await
                {
                    event!(Level::ERROR, %err, "Add err");
                }
                file_id
            }
        }
        Ok(GetAudioByURLKind::Empty) => {
            event!(Level::ERROR, "Audio not found");
            error::occured_in_chosen_inline_result(&bot, "Audio not found", inline_message_id, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Get info err");
            let text = format!(
                "Sorry, an error to get media info\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
    };
    if let Err(err) = edit_audio_by_id
        .execute(EditAudioByIdInput::new(inline_message_id, &file_id, &html_text_link("Link", url)))
        .await
    {
        event!(Level::ERROR, err = format_error_report(&err), "Edit err");
        let text = format!(
            "Sorry, an error to edit the message\n{}",
            expandable_blockquote(err.format(&bot.token))
        );
        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
    }
    Ok(EventReturn::Finish)
}
