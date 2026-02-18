use crate::{config::TimeoutsConfig, entities::MediaInFS, handlers_utils::send, interactors::Interactor};

use crate::utils::sanitize_send_filename;
use std::sync::Arc;
use telers::{
    errors::SessionErrorKind,
    methods,
    types::{InputFile, ReplyParameters},
    Bot,
};
use tracing::{debug, error, info, instrument};

pub struct SendVideo {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

pub struct SendVideoInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub media_in_fs: MediaInFS,
    pub name: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub with_delete: bool,
}

pub struct SendAudio {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

pub struct SendAudioInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub media_in_fs: MediaInFS,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub performer: Option<&'a str>,
    pub duration: Option<i64>,
    pub with_delete: bool,
}

impl Interactor<SendVideoInput<'_>> for &SendVideo {
    type Output = Box<str>;
    type Err = SessionErrorKind;

    #[instrument(skip_all, fields(%name, ?width, ?height, %with_delete, path = ?path.to_string_lossy()))]
    async fn execute(
        self,
        SendVideoInput {
            chat_id,
            reply_to_message_id,
            media_in_fs: MediaInFS {
                path,
                thumb_path,
                temp_dir,
            },
            name,
            width,
            height,
            duration,
            with_delete,
        }: SendVideoInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let send_name = sanitize_send_filename(path.as_ref(), name);

        debug!("Video sending");
        let message = send::with_retries(
            &self.bot,
            methods::SendVideo::new(chat_id, InputFile::fs_with_name(path, &send_name))
                .width_option(width)
                .height_option(height)
                .supports_streaming(true)
                .duration_option(duration)
                .disable_notification(true)
                .thumbnail_option(thumb_path.map(InputFile::fs))
                .reply_parameters_option(reply_to_message_id.map(|val| ReplyParameters::new(val).allow_sending_without_reply(true))),
            2,
            Some(self.timeouts_cfg.send_by_fs),
        )
        .await?;
        drop(temp_dir);
        let message_id = message.id();
        let file_id = match message.video() {
            Some(video) => video.file_id.clone(),
            None => message.document().unwrap().file_id.clone(),
        };
        drop(message);
        info!("Video sent");

        if with_delete {
            tokio::spawn({
                let bot = self.bot.clone();
                async move {
                    if let Err(err) = bot.send(methods::DeleteMessage::new(chat_id, message_id)).await {
                        error!(%err, "Delete message err");
                    }
                }
            });
        }

        Ok(file_id)
    }
}

impl Interactor<SendAudioInput<'_>> for &SendAudio {
    type Output = Box<str>;
    type Err = SessionErrorKind;

    #[instrument(skip_all, fields(name, uploader, with_delete))]
    async fn execute(
        self,
        SendAudioInput {
            chat_id,
            reply_to_message_id,
            media_in_fs: MediaInFS {
                path,
                thumb_path,
                temp_dir,
            },
            name,
            performer,
            title,
            duration,
            with_delete,
        }: SendAudioInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        let send_name = sanitize_send_filename(path.as_ref(), name);

        debug!("Audio sending");
        let message = send::with_retries(
            &self.bot,
            methods::SendAudio::new(chat_id, InputFile::fs_with_name(path, &send_name))
                .title_option(title)
                .duration_option(duration)
                .disable_notification(true)
                .performer_option(performer)
                .thumbnail_option(thumb_path.map(InputFile::fs))
                .reply_parameters_option(reply_to_message_id.map(|val| ReplyParameters::new(val).allow_sending_without_reply(true))),
            2,
            Some(self.timeouts_cfg.send_by_fs),
        )
        .await?;
        drop(temp_dir);
        let message_id = message.id();
        let file_id = message
            .audio()
            .map(|val| val.file_id.clone())
            .or(message.voice().map(|val| val.file_id.clone()))
            .unwrap();
        drop(message);
        info!("Audio sent");

        if with_delete {
            tokio::spawn({
                let bot = self.bot.clone();
                async move {
                    if let Err(err) = bot.send(methods::DeleteMessage::new(chat_id, message_id)).await {
                        error!(%err, "Delete message err");
                    }
                }
            });
        }

        Ok(file_id)
    }
}
