use crate::{
    entities::{AudioInFS, VideoInFS},
    handlers_utils::send,
    interactors::Interactor,
};

use telers::{
    errors::SessionErrorKind,
    methods::{SendAudio, SendVideo},
    types::InputFile,
    Bot,
};
use tracing::{event, instrument, Level};

const SEND_TIMEOUT: f32 = 180.0;

pub struct SendVideoInFS {
    bot: Bot,
}

impl SendVideoInFS {
    pub const fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

pub struct SendVideoInFSInput<'a> {
    pub chat_id: i64,
    pub video_in_fs: VideoInFS,
    pub name: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
}

impl Interactor for SendVideoInFS {
    type Input<'a> = SendVideoInFSInput<'a>;
    type Output = Box<str>;
    type Err = SessionErrorKind;

    #[instrument(skip(self))]
    async fn execute<'a>(
        &mut self,
        SendVideoInFSInput {
            chat_id,
            video_in_fs: VideoInFS { path, thumbnail_path },
            name,
            width,
            height,
            duration,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let message = send::with_retries(
            &self.bot,
            SendVideo::new(chat_id, InputFile::fs_with_name(path, name))
                .disable_notification(true)
                .width_option(width)
                .height_option(height)
                .duration_option(duration)
                .thumbnail_option(thumbnail_path.map(InputFile::fs))
                .supports_streaming(true),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;

        event!(Level::DEBUG, "Video sent");

        Ok(message.video().unwrap().file_id.clone())
    }
}

pub struct SendAudioInFS {
    bot: Bot,
}

impl SendAudioInFS {
    pub const fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

pub struct SendAudioInFSInput<'a> {
    pub chat_id: i64,
    pub audio_in_fs: AudioInFS,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub uploader: Option<&'a str>,
    pub duration: Option<i64>,
}

impl Interactor for SendAudioInFS {
    type Input<'a> = SendAudioInFSInput<'a>;
    type Output = Box<str>;
    type Err = SessionErrorKind;

    #[instrument(skip(self))]
    async fn execute<'a>(
        &mut self,
        SendAudioInFSInput {
            chat_id,
            audio_in_fs: AudioInFS { path, thumbnail_path },
            name,
            title,
            uploader,
            duration,
        }: Self::Input<'a>,
    ) -> Result<Self::Output, Self::Err> {
        let message = send::with_retries(
            &self.bot,
            SendAudio::new(chat_id, InputFile::fs_with_name(path, name))
                .disable_notification(true)
                .title_option(title)
                .duration_option(duration)
                .performer_option(uploader)
                .thumbnail_option(thumbnail_path.map(InputFile::fs)),
            2,
            Some(SEND_TIMEOUT),
        )
        .await?;

        event!(Level::DEBUG, "Audio sent");

        Ok(message.video().unwrap().file_id.clone())
    }
}
