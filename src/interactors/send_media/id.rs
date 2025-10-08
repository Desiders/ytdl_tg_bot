use crate::{
    entities::{TgAudioInPlaylist, TgVideoInPlaylist},
    handlers_utils::send,
    interactors::Interactor,
};

use std::sync::Arc;
use telers::{
    enums::ParseMode,
    errors::SessionErrorKind,
    methods::{EditMessageMedia, SendAudio, SendVideo},
    types::{InlineKeyboardMarkup, InputFile, InputMediaAudio, InputMediaVideo, ReplyParameters},
    Bot,
};
use tracing::{event, span, Level};

const SEND_TIMEOUT: f32 = 360.0;

pub struct SendVideoById {
    bot: Arc<Bot>,
}

impl SendVideoById {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct SendVideoByIdInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub id: &'a str,
}

impl Interactor for SendVideoById {
    type Input<'a> = SendVideoByIdInput<'a>;
    type Output = ();
    type Err = SessionErrorKind;

    async fn execute<'a>(
        &mut self,
        SendVideoByIdInput {
            chat_id,
            reply_to_message_id,
            id,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "send");
        let _guard = span.enter();

        event!(Level::DEBUG, "Video sending");
        send::with_retries(
            &self.bot,
            SendVideo::new(chat_id, InputFile::id(id))
                .reply_parameters_option(reply_to_message_id.map(ReplyParameters::new))
                .disable_notification(true)
                .supports_streaming(true),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::DEBUG, "Video sent");

        Ok(())
    }
}

pub struct SendAudioById {
    bot: Arc<Bot>,
}

impl SendAudioById {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct SendAudioByIdInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub id: &'a str,
}

impl Interactor for SendAudioById {
    type Input<'a> = SendAudioByIdInput<'a>;
    type Output = ();
    type Err = SessionErrorKind;

    async fn execute<'a>(
        &mut self,
        SendAudioByIdInput {
            chat_id,
            reply_to_message_id,
            id,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "send");
        let _guard = span.enter();

        event!(Level::DEBUG, "Audio sending");
        send::with_retries(
            &self.bot,
            SendAudio::new(chat_id, InputFile::id(id))
                .reply_parameters_option(reply_to_message_id.map(ReplyParameters::new))
                .disable_notification(true),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::DEBUG, "Audio sent");

        Ok(())
    }
}

pub struct EditVideoById {
    bot: Arc<Bot>,
}

impl EditVideoById {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct EditVideoByIdInput<'a> {
    pub inline_message_id: &'a str,
    pub id: &'a str,
    pub caption: &'a str,
}

impl<'a> EditVideoByIdInput<'a> {
    pub const fn new(inline_message_id: &'a str, id: &'a str, caption: &'a str) -> Self {
        Self {
            inline_message_id,
            id,
            caption,
        }
    }
}

impl Interactor for EditVideoById {
    type Input<'a> = EditVideoByIdInput<'a>;
    type Output = ();
    type Err = SessionErrorKind;

    async fn execute<'a>(
        &mut self,
        EditVideoByIdInput {
            inline_message_id,
            id,
            caption,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "send");
        let _guard = span.enter();

        event!(Level::DEBUG, "Video editing");
        send::with_retries(
            &self.bot,
            EditMessageMedia::new(InputMediaVideo::new(InputFile::id(id)).caption(caption).supports_streaming(true))
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::DEBUG, "Video edited");

        Ok(())
    }
}

pub struct EditAudioById {
    bot: Arc<Bot>,
}

impl EditAudioById {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct EditAudioByIdInput<'a> {
    pub inline_message_id: &'a str,
    pub id: &'a str,
    pub caption: &'a str,
}

impl<'a> EditAudioByIdInput<'a> {
    pub const fn new(inline_message_id: &'a str, id: &'a str, caption: &'a str) -> Self {
        Self {
            inline_message_id,
            id,
            caption,
        }
    }
}

impl Interactor for EditAudioById {
    type Input<'a> = EditAudioByIdInput<'a>;
    type Output = ();
    type Err = SessionErrorKind;

    async fn execute<'a>(
        &mut self,
        EditAudioByIdInput {
            inline_message_id,
            id,
            caption,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "send");
        let _guard = span.enter();

        event!(Level::DEBUG, "Audio editing");
        send::with_retries(
            &self.bot,
            EditMessageMedia::new(InputMediaAudio::new(InputFile::id(id)).caption(caption).parse_mode(ParseMode::HTML))
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::DEBUG, "Audio edited");

        Ok(())
    }
}

pub struct SendVideoPlaylistById {
    bot: Arc<Bot>,
}

impl SendVideoPlaylistById {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct SendVideoPlaylistByIdInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub videos: Vec<TgVideoInPlaylist>,
}

impl SendVideoPlaylistByIdInput {
    pub const fn new(chat_id: i64, reply_to_message_id: Option<i64>, videos: Vec<TgVideoInPlaylist>) -> Self {
        Self {
            chat_id,
            reply_to_message_id,
            videos,
        }
    }
}

impl Interactor for SendVideoPlaylistById {
    type Input<'a> = SendVideoPlaylistByIdInput;
    type Output = ();
    type Err = SessionErrorKind;

    async fn execute<'a>(
        &mut self,
        SendVideoPlaylistByIdInput {
            chat_id,
            reply_to_message_id,
            mut videos,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "send_playlist");
        let _guard = span.enter();

        videos.sort_by(|a, b| a.index.cmp(&b.index));

        event!(Level::DEBUG, "Video playlist sending");
        send::media_groups(
            &self.bot,
            chat_id,
            videos
                .into_iter()
                .map(|TgVideoInPlaylist { file_id, .. }| InputMediaVideo::new(InputFile::id(file_id.into_string())))
                .collect(),
            reply_to_message_id,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::DEBUG, "Video playlist sent");

        Ok(())
    }
}

pub struct SendAudioPlaylistById {
    bot: Arc<Bot>,
}

impl SendAudioPlaylistById {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct SendAudioPlaylistByIdInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub audios: Vec<TgAudioInPlaylist>,
}

impl SendAudioPlaylistByIdInput {
    pub const fn new(chat_id: i64, reply_to_message_id: Option<i64>, audios: Vec<TgAudioInPlaylist>) -> Self {
        Self {
            chat_id,
            reply_to_message_id,
            audios,
        }
    }
}

impl Interactor for SendAudioPlaylistById {
    type Input<'a> = SendAudioPlaylistByIdInput;
    type Output = ();
    type Err = SessionErrorKind;

    async fn execute<'a>(
        &mut self,
        SendAudioPlaylistByIdInput {
            chat_id,
            reply_to_message_id,
            mut audios,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let span = span!(Level::INFO, "send_playlist");
        let _guard = span.enter();

        audios.sort_by(|a, b| a.index.cmp(&b.index));

        event!(Level::DEBUG, "Audio playlist sending");
        send::media_groups(
            &self.bot,
            chat_id,
            audios
                .into_iter()
                .map(|TgAudioInPlaylist { file_id, .. }| InputMediaAudio::new(InputFile::id(file_id.into_string())))
                .collect(),
            reply_to_message_id,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::DEBUG, "Audio playlist sent");

        Ok(())
    }
}
