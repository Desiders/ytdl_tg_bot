use crate::{
    entities::MediaInPlaylist,
    interactors::Interactor,
    services::messenger::{
        EditMediaByIdRequest, MediaGroupItem, MessengerError, MessengerPort, SendMediaByIdRequest, SendMediaGroupRequest,
    },
};

use std::sync::Arc;
use url::Url;

pub struct SendMediaInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub id: &'a str,
    pub webpage_url: Option<&'a Url>,
    pub link_is_visible: bool,
}

pub struct SendPlaylistInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub playlist: Vec<MediaInPlaylist>,
    pub link_is_visible: bool,
}

pub struct EditMediaInput<'a> {
    pub inline_message_id: &'a str,
    pub id: &'a str,
    pub webpage_url: Option<&'a Url>,
    pub link_is_visible: bool,
}

pub struct SendVideo<Messenger> {
    pub messenger: Arc<Messenger>,
}

impl<Messenger> Interactor<SendMediaInput<'_>> for &SendVideo<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: SendMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .send_video_by_id(SendMediaByIdRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                remote_id: input.id,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}

pub struct SendAudio<Messenger> {
    pub messenger: Arc<Messenger>,
}

impl<Messenger> Interactor<SendMediaInput<'_>> for &SendAudio<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: SendMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .send_audio_by_id(SendMediaByIdRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                remote_id: input.id,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}

pub struct EditVideo<Messenger> {
    pub messenger: Arc<Messenger>,
}

impl<Messenger> Interactor<EditMediaInput<'_>> for &EditVideo<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: EditMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .edit_video_by_id(EditMediaByIdRequest {
                inline_message_id: input.inline_message_id,
                remote_id: input.id,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}

pub struct EditAudio<Messenger> {
    pub messenger: Arc<Messenger>,
}

impl<Messenger> Interactor<EditMediaInput<'_>> for &EditAudio<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: EditMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .edit_audio_by_id(EditMediaByIdRequest {
                inline_message_id: input.inline_message_id,
                remote_id: input.id,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}

pub struct SendVideoPlaylist<Messenger> {
    pub messenger: Arc<Messenger>,
}

impl<Messenger> Interactor<SendPlaylistInput> for &SendVideoPlaylist<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: SendPlaylistInput) -> Result<Self::Output, Self::Err> {
        self.messenger
            .send_video_group(SendMediaGroupRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                items: input
                    .playlist
                    .into_iter()
                    .map(|item| MediaGroupItem {
                        remote_id: item.file_id.into(),
                        webpage_url: item.webpage_url,
                    })
                    .collect(),
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}

pub struct SendAudioPlaylist<Messenger> {
    pub messenger: Arc<Messenger>,
}

impl<Messenger> Interactor<SendPlaylistInput> for &SendAudioPlaylist<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: SendPlaylistInput) -> Result<Self::Output, Self::Err> {
        self.messenger
            .send_audio_group(SendMediaGroupRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                items: input
                    .playlist
                    .into_iter()
                    .map(|item| MediaGroupItem {
                        remote_id: item.file_id.into(),
                        webpage_url: item.webpage_url,
                    })
                    .collect(),
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}
