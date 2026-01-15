use crate::{
    config::TimeoutsConfig,
    entities::{AudioInFS, VideoInFS},
    handlers_utils::send,
    interactors::Interactor,
};

use crate::utils::sanitize_send_filename;
use std::sync::Arc;
use telers::{
    errors::SessionErrorKind,
    methods::{DeleteMessage, SendAudio, SendVideo},
    types::{InputFile, ReplyParameters},
    Bot,
};
use tracing::{debug, error, info, instrument};

pub struct SendVideoInFS {
    bot: Arc<Bot>,
    timeouts_cfg: Arc<TimeoutsConfig>,
}

impl SendVideoInFS {
    pub const fn new(bot: Arc<Bot>, timeouts_cfg: Arc<TimeoutsConfig>) -> Self {
        Self { bot, timeouts_cfg }
    }
}

pub struct SendVideoInFSInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub video_in_fs: VideoInFS,
    pub name: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub with_delete: bool,
}

impl<'a> SendVideoInFSInput<'a> {
    pub const fn new(
        chat_id: i64,
        reply_to_message_id: Option<i64>,
        video_in_fs: VideoInFS,
        name: &'a str,
        width: Option<i64>,
        height: Option<i64>,
        duration: Option<i64>,
        with_delete: bool,
    ) -> Self {
        Self {
            chat_id,
            reply_to_message_id,
            video_in_fs,
            name,
            width,
            height,
            duration,
            with_delete,
        }
    }
}

pub struct SendAudioInFS {
    bot: Arc<Bot>,
    timeouts_cfg: Arc<TimeoutsConfig>,
}

impl SendAudioInFS {
    pub const fn new(bot: Arc<Bot>, timeouts_cfg: Arc<TimeoutsConfig>) -> Self {
        Self { bot, timeouts_cfg }
    }
}

pub struct SendAudioInFSInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub audio_in_fs: AudioInFS,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub performer: Option<&'a str>,
    pub duration: Option<i64>,
    pub with_delete: bool,
}

impl<'a> SendAudioInFSInput<'a> {
    pub const fn new(
        chat_id: i64,
        reply_to_message_id: Option<i64>,
        audio_in_fs: AudioInFS,
        name: &'a str,
        title: Option<&'a str>,
        performer: Option<&'a str>,
        duration: Option<i64>,
        with_delete: bool,
    ) -> Self {
        Self {
            chat_id,
            reply_to_message_id,
            audio_in_fs,
            name,
            performer,
            title,
            duration,
            with_delete,
        }
    }
}

impl Interactor<SendVideoInFSInput<'_>> for &SendVideoInFS {
    type Output = Box<str>;
    type Err = SessionErrorKind;

    #[instrument(skip_all, fields(name, width, height, with_delete))]
    async fn execute(
        self,
        SendVideoInFSInput {
            chat_id,
            reply_to_message_id,
            video_in_fs: VideoInFS {
                path,
                thumbnail_path,
                temp_dir,
            },
            name,
            width,
            height,
            duration,
            with_delete,
        }: SendVideoInFSInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Video sending");

        let send_name = sanitize_send_filename(path.as_ref(), name);

        let message = send::with_retries(
            &self.bot,
            SendVideo::new(chat_id, InputFile::fs_with_name(path, &send_name))
                .disable_notification(true)
                .width_option(width)
                .height_option(height)
                .duration_option(duration)
                .thumbnail_option(thumbnail_path.map(InputFile::fs))
                .supports_streaming(true)
                .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true))),
            2,
            Some(self.timeouts_cfg.send_by_fs),
        )
        .await?;
        let message_id = message.id();
        let file_id = message.video().unwrap().file_id.clone();
        drop(message);
        drop(temp_dir);

        info!("Video sent");

        if with_delete {
            tokio::spawn({
                let bot = self.bot.clone();
                async move {
                    if let Err(err) = bot.send(DeleteMessage::new(chat_id, message_id)).await {
                        error!(%err, "Delete message err");
                    }
                }
            });
        }

        Ok(file_id)
    }
}

impl Interactor<SendAudioInFSInput<'_>> for &SendAudioInFS {
    type Output = Box<str>;
    type Err = SessionErrorKind;

    #[instrument(skip_all, fields(name, uploader, with_delete))]
    async fn execute(
        self,
        SendAudioInFSInput {
            chat_id,
            reply_to_message_id,
            audio_in_fs: AudioInFS {
                path,
                temp_dir,
                thumbnail_path,
            },
            name,
            performer,
            title,
            duration,
            with_delete,
        }: SendAudioInFSInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Audio sending");

        let send_name = sanitize_send_filename(path.as_ref(), name);

        let method = SendAudio::new(chat_id, InputFile::fs_with_name(path, &send_name))
            .disable_notification(true)
            .duration_option(duration)
            .thumbnail_option(thumbnail_path.map(InputFile::fs))
            .title_option(title)
            .performer_option(performer)
            .reply_parameters_option(reply_to_message_id.map(|r| ReplyParameters::new(r).allow_sending_without_reply(true)));

        let message = send::with_retries(&self.bot, method, 2, Some(self.timeouts_cfg.send_by_fs)).await?;
        let message_id = message.id();
        let file_id = message.audio().unwrap().file_id.clone();
        drop(message);
        drop(temp_dir);

        info!("Audio sent");

        if with_delete {
            tokio::spawn({
                let bot = self.bot.clone();
                async move {
                    if let Err(err) = bot.send(DeleteMessage::new(chat_id, message_id)).await {
                        error!(%err, "Delete message err");
                    }
                }
            });
        }

        Ok(file_id)
    }
}
