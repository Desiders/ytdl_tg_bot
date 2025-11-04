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
use tracing::{event, instrument, Level};

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

impl<'a> SendVideoByIdInput<'a> {
    pub const fn new(chat_id: i64, reply_to_message_id: Option<i64>, id: &'a str) -> Self {
        Self {
            chat_id,
            reply_to_message_id,
            id,
        }
    }
}

impl Interactor<SendVideoByIdInput<'_>> for &SendVideoById {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendVideoByIdInput {
            chat_id,
            reply_to_message_id,
            id,
        }: SendVideoByIdInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        event!(Level::DEBUG, "Video sending");
        send::with_retries(
            &self.bot,
            SendVideo::new(chat_id, InputFile::id(id))
                .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)))
                .disable_notification(true)
                .supports_streaming(true),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::INFO, "Video sent");

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

impl<'a> SendAudioByIdInput<'a> {
    pub const fn new(chat_id: i64, reply_to_message_id: Option<i64>, id: &'a str) -> Self {
        Self {
            chat_id,
            reply_to_message_id,
            id,
        }
    }
}

impl Interactor<SendAudioByIdInput<'_>> for &SendAudioById {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendAudioByIdInput {
            chat_id,
            reply_to_message_id,
            id,
        }: SendAudioByIdInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        event!(Level::DEBUG, "Audio sending");
        send::with_retries(
            &self.bot,
            SendAudio::new(chat_id, InputFile::id(id))
                .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)))
                .disable_notification(true),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::INFO, "Audio sent");

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

impl Interactor<EditVideoByIdInput<'_>> for &EditVideoById {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        EditVideoByIdInput {
            inline_message_id,
            id,
            caption,
        }: EditVideoByIdInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        event!(Level::DEBUG, "Video editing");
        send::with_retries(
            &self.bot,
            EditMessageMedia::new(
                InputMediaVideo::new(InputFile::id(id))
                    .caption(caption)
                    .supports_streaming(true)
                    .parse_mode(ParseMode::HTML),
            )
            .inline_message_id(inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::INFO, "Video edited");

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

impl Interactor<EditAudioByIdInput<'_>> for &EditAudioById {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        EditAudioByIdInput {
            inline_message_id,
            id,
            caption,
        }: EditAudioByIdInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
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
        event!(Level::INFO, "Audio edited");

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
    pub playlist: Vec<TgVideoInPlaylist>,
}

impl SendVideoPlaylistByIdInput {
    pub fn new(chat_id: i64, reply_to_message_id: Option<i64>, mut playlist: Vec<TgVideoInPlaylist>) -> Self {
        playlist.sort_by_key(|TgVideoInPlaylist { index, .. }| *index);
        Self {
            chat_id,
            reply_to_message_id,
            playlist,
        }
    }
}

impl Interactor<SendVideoPlaylistByIdInput> for &SendVideoPlaylistById {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendVideoPlaylistByIdInput {
            chat_id,
            reply_to_message_id,
            playlist,
        }: SendVideoPlaylistByIdInput,
    ) -> Result<Self::Output, Self::Err> {
        event!(Level::DEBUG, "Video playlist sending");
        send::media_groups(
            &self.bot,
            chat_id,
            playlist
                .into_iter()
                .map(|TgVideoInPlaylist { file_id, .. }| InputMediaVideo::new(InputFile::id(file_id.into_string())))
                .collect(),
            reply_to_message_id,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::INFO, "Video playlist sent");

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
    pub playlist: Vec<TgAudioInPlaylist>,
}

impl SendAudioPlaylistByIdInput {
    pub fn new(chat_id: i64, reply_to_message_id: Option<i64>, mut playlist: Vec<TgAudioInPlaylist>) -> Self {
        playlist.sort_by_key(|TgAudioInPlaylist { index, .. }| *index);
        Self {
            chat_id,
            reply_to_message_id,
            playlist,
        }
    }
}

impl Interactor<SendAudioPlaylistByIdInput> for &SendAudioPlaylistById {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendAudioPlaylistByIdInput {
            chat_id,
            reply_to_message_id,
            playlist,
        }: SendAudioPlaylistByIdInput,
    ) -> Result<Self::Output, Self::Err> {
        event!(Level::DEBUG, "Audio playlist sending");
        send::media_groups(
            &self.bot,
            chat_id,
            playlist
                .into_iter()
                .map(|TgAudioInPlaylist { file_id, .. }| InputMediaAudio::new(InputFile::id(file_id.into_string())))
                .collect(),
            reply_to_message_id,
            Some(SEND_TIMEOUT),
        )
        .await?;
        event!(Level::INFO, "Audio playlist sent");

        Ok(())
    }
}
