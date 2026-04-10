use crate::{
    entities::MediaForUpload,
    interactors::Interactor,
    services::messenger::{MessengerError, MessengerPort, UploadAudioRequest, UploadVideoRequest},
};

use std::sync::Arc;
use url::Url;

pub struct SendVideo<Messenger> {
    pub messenger: Arc<Messenger>,
}

pub struct SendVideoInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub media_for_upload: MediaForUpload,
    pub name: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub with_delete: bool,
    pub webpage_url: &'a Url,
    pub link_is_visible: bool,
}

pub struct SendAudio<Messenger> {
    pub messenger: Arc<Messenger>,
}

pub struct SendAudioInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub media_for_upload: MediaForUpload,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub performer: Option<&'a str>,
    pub duration: Option<i64>,
    pub with_delete: bool,
    pub webpage_url: &'a Url,
    pub link_is_visible: bool,
}

impl<Messenger> Interactor<SendVideoInput<'_>> for &SendVideo<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = String;
    type Err = MessengerError;

    async fn execute(self, input: SendVideoInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .upload_video(UploadVideoRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                media_for_upload: input.media_for_upload,
                name: input.name,
                width: input.width,
                height: input.height,
                duration: input.duration,
                with_delete: input.with_delete,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
            .map(Into::into)
    }
}

impl<Messenger> Interactor<SendAudioInput<'_>> for &SendAudio<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = String;
    type Err = MessengerError;

    async fn execute(self, input: SendAudioInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .upload_audio(UploadAudioRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                media_for_upload: input.media_for_upload,
                name: input.name,
                title: input.title,
                performer: input.performer,
                duration: input.duration,
                with_delete: input.with_delete,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
            .map(Into::into)
    }
}
