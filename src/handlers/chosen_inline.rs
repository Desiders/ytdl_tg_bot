use crate::{
    config::{ChatConfig, YtDlpConfig},
    entities::{AudioAndFormat, PreferredLanguages, Range, UrlWithParams, VideoAndFormat},
    handlers_utils::error,
    interactors::{
        download::{DownloadAudio, DownloadAudioInput, DownloadVideo, DownloadVideoInput},
        send_media::{
            EditAudioById, EditAudioByIdInput, EditVideoById, EditVideoByIdInput, SendAudioInFS, SendAudioInFSInput, SendVideoInFS,
            SendVideoInFSInput,
        },
        GetMedaInfoByURLInput, GetMediaInfoById, GetMediaInfoByIdInput, GetMediaInfoByURL, Interactor as _,
    },
    utils::{format_error_report, FormatErrorToMessage as _},
};

use froodi::async_impl::Container;
use std::str::FromStr as _;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    types::ChosenInlineResult,
    utils::text::{html_formatter::expandable_blockquote, html_text_link},
    Bot, Extension,
};
use tracing::{event, span, Level};
use url::{Host, Url};

pub async fn download_by_url(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        ..
    }: ChosenInlineResult,
    Extension(UrlWithParams { url, params }): Extension<UrlWithParams>,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(container): Extension<Container>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let is_video = result_id.starts_with("video_");

    let span = span!(
        Level::INFO,
        "chosen_inline_url_handler",
        inline_message_id,
        result_id,
        url = url.as_str(),
        ?params
    );
    let _enter = span.enter();

    event!(Level::DEBUG, "Got url");

    let mut get_media_info = container.get_transient::<GetMediaInfoByURL>().await.unwrap();
    let mut download_video = container.get_transient::<DownloadVideo>().await.unwrap();
    let mut download_audio = container.get_transient::<DownloadAudio>().await.unwrap();
    let mut send_video_in_fs = container.get_transient::<SendVideoInFS>().await.unwrap();
    let mut edit_video_by_id = container.get_transient::<EditVideoById>().await.unwrap();
    let mut send_audio_in_fs = container.get_transient::<SendAudioInFS>().await.unwrap();
    let mut edit_audio_by_id = container.get_transient::<EditAudioById>().await.unwrap();

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
            Ok((_, file_id)) => file_id,
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
        Ok((_, file_id)) => file_id,
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

pub async fn download_by_text(
    bot: Bot,
    ChosenInlineResult {
        result_id,
        inline_message_id,
        ..
    }: ChosenInlineResult,
    Extension(yt_dlp_cfg): Extension<YtDlpConfig>,
    Extension(chat_cfg): Extension<ChatConfig>,
    Extension(container): Extension<Container>,
) -> HandlerResult {
    let inline_message_id = inline_message_id.as_deref().unwrap();
    let (result_prefix, video_id) = result_id.split_once('_').unwrap();
    let is_video = result_prefix.starts_with("video");

    let span = span!(Level::INFO, "chosen_inline_text_handler", inline_message_id, result_id);
    let _enter = span.enter();

    event!(Level::DEBUG, "Got url");

    let mut get_media_info = container.get_transient::<GetMediaInfoById>().await.unwrap();
    let mut download_video = container.get_transient::<DownloadVideo>().await.unwrap();
    let mut download_audio = container.get_transient::<DownloadAudio>().await.unwrap();
    let mut send_video_in_fs = container.get_transient::<SendVideoInFS>().await.unwrap();
    let mut edit_video_by_id = container.get_transient::<EditVideoById>().await.unwrap();
    let mut send_audio_in_fs = container.get_transient::<SendAudioInFS>().await.unwrap();
    let mut edit_audio_by_id = container.get_transient::<EditAudioById>().await.unwrap();

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
            Ok((_, file_id)) => file_id,
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
        Ok((_, file_id)) => file_id,
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
