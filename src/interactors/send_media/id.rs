use crate::{
    entities::{TgAudioInPlaylist, TgVideoInPlaylist},
    handlers_utils::send,
    interactors::Interactor,
};

use telers::{
    errors::SessionErrorKind,
    methods::{SendAudio, SendVideo},
    types::{InputFile, InputMediaAudio, InputMediaVideo, ReplyParameters},
    Bot,
};
use tracing::{event, instrument, Level};

const SEND_TIMEOUT: f32 = 360.0;

pub struct SendVideoById {
    bot: Bot,
}

impl SendVideoById {
    pub const fn new(bot: Bot) -> Self {
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

    #[instrument(skip(self))]
    async fn execute<'a>(
        &mut self,
        SendVideoByIdInput {
            chat_id,
            reply_to_message_id,
            id,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
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
    bot: Bot,
}

impl SendAudioById {
    pub const fn new(bot: Bot) -> Self {
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

    #[instrument(skip(self))]
    async fn execute<'a>(
        &mut self,
        SendAudioByIdInput {
            chat_id,
            reply_to_message_id,
            id,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
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

pub struct SendVideoPlaylistById {
    bot: Bot,
}

impl SendVideoPlaylistById {
    pub const fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

pub struct SendVideoPlaylistByIdInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub videos: Vec<TgVideoInPlaylist>,
}

impl Interactor for SendVideoPlaylistById {
    type Input<'a> = SendVideoPlaylistByIdInput;
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip(self))]
    async fn execute<'a>(
        &mut self,
        SendVideoPlaylistByIdInput {
            chat_id,
            reply_to_message_id,
            mut videos,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        videos.sort_by(|a, b| a.index.cmp(&b.index));

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
    bot: Bot,
}

impl SendAudioPlaylistById {
    pub const fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

pub struct SendAudioPlaylistByIdInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub audios: Vec<TgAudioInPlaylist>,
}

impl Interactor for SendAudioPlaylistById {
    type Input<'a> = SendAudioPlaylistByIdInput;
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip(self))]
    async fn execute<'a>(
        &mut self,
        SendAudioPlaylistByIdInput {
            chat_id,
            reply_to_message_id,
            mut audios,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        audios.sort_by(|a, b| a.index.cmp(&b.index));

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
