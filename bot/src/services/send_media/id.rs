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
    pub caption: Option<&'a str>,
}

pub struct SendPlaylistInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub playlist: Vec<MediaInPlaylist>,
    pub link_is_visible: bool,
    pub caption: Option<String>,
}

pub struct EditMediaInput<'a> {
    pub inline_message_id: &'a str,
    pub id: &'a str,
    pub webpage_url: Option<&'a Url>,
    pub link_is_visible: bool,
}

pub struct SendVideo<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> SendVideo<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
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
                caption: input.caption,
            })
            .await
    }
}

pub struct SendAudio<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> SendAudio<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
}

pub struct SendPhoto<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> SendPhoto<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
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
                caption: input.caption,
            })
            .await
    }
}

impl<Messenger> Interactor<SendMediaInput<'_>> for &SendPhoto<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: SendMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .send_photo_by_id(SendMediaByIdRequest {
                chat_id: input.chat_id,
                reply_to_message_id: input.reply_to_message_id,
                remote_id: input.id,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
                caption: input.caption,
            })
            .await
    }
}

pub struct EditVideo<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> EditVideo<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
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
    messenger: Arc<Messenger>,
}

impl<Messenger> EditAudio<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
}

pub struct EditPhoto<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> EditPhoto<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
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

impl<Messenger> Interactor<EditMediaInput<'_>> for &EditPhoto<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: EditMediaInput<'_>) -> Result<Self::Output, Self::Err> {
        self.messenger
            .edit_photo_by_id(EditMediaByIdRequest {
                inline_message_id: input.inline_message_id,
                remote_id: input.id,
                webpage_url: input.webpage_url,
                link_is_visible: input.link_is_visible,
            })
            .await
    }
}

pub struct SendVideoPlaylist<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> SendVideoPlaylist<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
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
                caption: input.caption,
            })
            .await
    }
}

pub struct SendAudioPlaylist<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> SendAudioPlaylist<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
}

pub struct SendPhotoPlaylist<Messenger> {
    messenger: Arc<Messenger>,
}

impl<Messenger> SendPhotoPlaylist<Messenger> {
    #[must_use]
    pub const fn new(messenger: Arc<Messenger>) -> Self {
        Self { messenger }
    }
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
                caption: input.caption,
            })
            .await
    }
}

impl<Messenger> Interactor<SendPlaylistInput> for &SendPhotoPlaylist<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = MessengerError;

    async fn execute(self, input: SendPlaylistInput) -> Result<Self::Output, Self::Err> {
        self.messenger
            .send_photo_group(SendMediaGroupRequest {
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
                caption: input.caption,
            })
            .await
    }
}
