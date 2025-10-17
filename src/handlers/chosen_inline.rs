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
        AddDownloadedAudio, AddDownloadedMediaInput, AddDownloadedVideo, GetMedaInfoByURLInput, GetMediaInfoById, GetMediaInfoByIdInput,
        GetMediaInfoByURL, Interactor as _,
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
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
    InjectTransient(mut add_downloaded_video): InjectTransient<AddDownloadedVideo>,
    InjectTransient(mut add_downloaded_audio): InjectTransient<AddDownloadedAudio>,
    InjectTransient(mut get_media_info): InjectTransient<GetMediaInfoByURL>,
    InjectTransient(mut download_video): InjectTransient<DownloadVideo>,
    InjectTransient(mut download_audio): InjectTransient<DownloadAudio>,
    InjectTransient(mut send_video_in_fs): InjectTransient<SendVideoInFS>,
    InjectTransient(mut edit_video_by_id): InjectTransient<EditVideoById>,
    InjectTransient(mut send_audio_in_fs): InjectTransient<SendAudioInFS>,
    InjectTransient(mut edit_audio_by_id): InjectTransient<EditAudioById>,
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
    let mut videos = match get_media_info.execute(GetMedaInfoByURLInput::new(&url, &Range::default())).await {
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
        if let Err(err) = add_downloaded_video
            .execute(AddDownloadedMediaInput::new(file_id, video.id.into_boxed_str(), &mut tx_manager))
            .await
        {
            event!(Level::ERROR, %err, "Add downloaded video err");
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
    if let Err(err) = add_downloaded_audio
        .execute(AddDownloadedMediaInput::new(file_id, video.id.into_boxed_str(), &mut tx_manager))
        .await
    {
        event!(Level::ERROR, %err, "Add downloaded audio err");
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
    InjectTransient(mut tx_manager): InjectTransient<TxManager>,
    InjectTransient(mut add_downloaded_video): InjectTransient<AddDownloadedVideo>,
    InjectTransient(mut add_downloaded_audio): InjectTransient<AddDownloadedAudio>,
    InjectTransient(mut get_media_info): InjectTransient<GetMediaInfoById>,
    InjectTransient(mut download_video): InjectTransient<DownloadVideo>,
    InjectTransient(mut download_audio): InjectTransient<DownloadAudio>,
    InjectTransient(mut send_video_in_fs): InjectTransient<SendVideoInFS>,
    InjectTransient(mut edit_video_by_id): InjectTransient<EditVideoById>,
    InjectTransient(mut send_audio_in_fs): InjectTransient<SendAudioInFS>,
    InjectTransient(mut edit_audio_by_id): InjectTransient<EditAudioById>,
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
        if let Err(err) = add_downloaded_video
            .execute(AddDownloadedMediaInput::new(file_id, video.id.into_boxed_str(), &mut tx_manager))
            .await
        {
            event!(Level::ERROR, %err, "Add downloaded video err");
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
    if let Err(err) = add_downloaded_audio
        .execute(AddDownloadedMediaInput::new(file_id, video.id.into_boxed_str(), &mut tx_manager))
        .await
    {
        event!(Level::ERROR, %err, "Add downloaded audio err");
    }
    Ok(EventReturn::Finish)
}
