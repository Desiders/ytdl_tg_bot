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
        GetAudioByURL, GetAudioByURLInput, GetAudioByURLKind, GetMediaInfoById, GetMediaInfoByIdInput, GetVideoByURL, GetVideoByURLInput,
        GetVideoByURLKind, Interactor as _,
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
use url::{Host, Url};

#[instrument(skip_all, fields(inline_message_id, is_video, url = url.as_str(), ?params))]
pub async fn download_by_url(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
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
    InjectTransient(tx_manager): InjectTransient<TxManager>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let is_video = result_id.starts_with("video_");

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
            .execute(GetVideoByURLInput::new(&url, &Range::default(), tx_manager))
            .await
        {
            Ok(GetVideoByURLKind::SingleCached(file_id)) => file_id.into_boxed_str(),
            Ok(GetVideoByURLKind::PlaylistCached(mut playlist)) => playlist.remove(0).file_id,
            Ok(GetVideoByURLKind::SingleUncached(media)) => {
                let media_and_format = match VideoAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages)
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
                let media_in_fs = match download_video.execute(DownloadVideoInput::new(&url, media_and_format)).await {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Download err");
                        let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                match send_video_in_fs
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
                }
            }
            Ok(GetVideoByURLKind::PlaylistUncached(mut playlist)) => {
                let media = playlist.remove(0);
                let media_and_format = match VideoAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages)
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
                let media_in_fs = match download_video.execute(DownloadVideoInput::new(&url, media_and_format)).await {
                    Ok(val) => val,
                    Err(err) => {
                        event!(Level::ERROR, err = format_error_report(&err), "Download err");
                        let text = format!("Sorry, an error to download\n{}", expandable_blockquote(err.format(&bot.token)));
                        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                        return Ok(EventReturn::Finish);
                    }
                };
                match send_video_in_fs
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
        .execute(GetAudioByURLInput::new(&url, &Range::default(), tx_manager))
        .await
    {
        Ok(GetAudioByURLKind::SingleCached(file_id)) => file_id.into_boxed_str(),
        Ok(GetAudioByURLKind::PlaylistCached(mut playlist)) => playlist.remove(0).file_id,
        Ok(GetAudioByURLKind::SingleUncached(media)) => {
            let media_and_format = match AudioAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
            match send_audio_in_fs
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
            }
        }
        Ok(GetAudioByURLKind::PlaylistUncached(mut playlist)) => {
            let media = playlist.remove(0);
            let media_and_format = match AudioAndFormat::new_with_select_format(&media, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
            match send_audio_in_fs
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
        ..
    }: ChosenInlineResult,
    Inject(yt_dlp_cfg): Inject<YtDlpConfig>,
    Inject(chat_cfg): Inject<ChatConfig>,
    Inject(get_media_info): Inject<GetMediaInfoById>,
    Inject(download_video): Inject<DownloadVideo>,
    Inject(download_audio): Inject<DownloadAudio>,
    Inject(send_video_in_fs): Inject<SendVideoInFS>,
    Inject(edit_video_by_id): Inject<EditVideoById>,
    Inject(send_audio_in_fs): Inject<SendAudioInFS>,
    Inject(edit_audio_by_id): Inject<EditAudioById>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let (result_prefix, video_id) = result_id.split_once('_').unwrap();
    let is_video = result_prefix.starts_with("video");

    Span::current()
        .record("inline_message_id", inline_message_id)
        .record("is_video", is_video)
        .record("video_id", video_id);

    event!(Level::DEBUG, "Got url");

    let preferred_languages = PreferredLanguages::default();
    let mut videos = match get_media_info
        .execute(GetMediaInfoByIdInput::new(
            video_id,
            &Host::Domain("youtube.com"),
            &Range::default(),
        ))
        .await
    {
        Ok(val) => val,
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
    if videos.is_empty() {
        event!(Level::ERROR, "Video not found");
        error::occured_in_chosen_inline_result(&bot, "Video not found", inline_message_id, Some(ParseMode::HTML)).await?;
        return Ok(EventReturn::Finish);
    }
    let video = videos.remove(0);
    drop(videos);
    let url = Url::parse(&video.original_url).unwrap();

    if is_video {
        let video_and_format = match VideoAndFormat::new_with_select_format(&video, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
        let video_in_fs = match download_video.execute(DownloadVideoInput::new(&url, video_and_format)).await {
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Download video err");
                let text = format!(
                    "Sorry, an error to download the video\n{}",
                    expandable_blockquote(err.format(&bot.token))
                );
                error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
                return Ok(EventReturn::Finish);
            }
        };
        let file_id = match send_video_in_fs
            .execute(SendVideoInFSInput::new(
                chat_cfg.receiver_chat_id,
                None,
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
            Ok(val) => val,
            Err(err) => {
                event!(Level::ERROR, err = format_error_report(&err), "Send video err");
                let text = format!(
                    "Sorry, an error to download the video\n{}",
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
            event!(Level::ERROR, err = format_error_report(&err), "Edit video err");
            let text = format!(
                "Sorry, an error to edit the video\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
        }
        return Ok(EventReturn::Finish);
    }

    let audio_and_format = match AudioAndFormat::new_with_select_format(&video, yt_dlp_cfg.max_file_size, &preferred_languages) {
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
    let audio_in_fs = match download_audio.execute(DownloadAudioInput::new(&url, audio_and_format)).await {
        Ok(val) => val,
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Download audio err");
            let text = format!(
                "Sorry, an error to download the audio\n{}",
                expandable_blockquote(err.format(&bot.token))
            );
            error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
            return Ok(EventReturn::Finish);
        }
    };
    let file_id = match send_audio_in_fs
        .execute(SendAudioInFSInput::new(
            chat_cfg.receiver_chat_id,
            None,
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
            let text = format!(
                "Sorry, an error to download the audio\n{}",
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
        event!(Level::ERROR, err = format_error_report(&err), "Edit audio err");
        let text = format!(
            "Sorry, an error to edit the audio\n{}",
            expandable_blockquote(err.format(&bot.token))
        );
        error::occured_in_chosen_inline_result(&bot, &text, inline_message_id, Some(ParseMode::HTML)).await?;
    }
    Ok(EventReturn::Finish)
}
