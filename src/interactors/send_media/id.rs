use crate::{config::TimeoutsConfig, entities::MediaInPlaylist, handlers_utils::send, interactors::Interactor};

use std::sync::Arc;
use telers::{
    enums::ParseMode,
    errors::SessionErrorKind,
    methods,
    types::{InlineKeyboardMarkup, InputFile, InputMediaAudio, InputMediaVideo, ReplyParameters},
    Bot,
};
use tracing::{debug, info, instrument};

pub struct SendMediaInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub id: &'a str,
}

pub struct SendPlaylistInput {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub playlist: Vec<MediaInPlaylist>,
}

pub struct EditMediaInput<'a> {
    pub inline_message_id: &'a str,
    pub id: &'a str,
    pub caption: &'a str,
}

pub struct SendVideo {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

impl Interactor<SendMediaInput<'_>> for &SendVideo {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendMediaInput {
            chat_id,
            reply_to_message_id,
            id,
        }: SendMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Video sending");
        send::with_retries(
            &self.bot,
            methods::SendVideo::new(chat_id, InputFile::id(id))
                .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)))
                .disable_notification(true)
                .supports_streaming(true),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        info!("Video sent");

        Ok(())
    }
}

pub struct SendAudio {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

impl Interactor<SendMediaInput<'_>> for &SendAudio {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendMediaInput {
            chat_id,
            reply_to_message_id,
            id,
        }: SendMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Audio sending");
        send::with_retries(
            &self.bot,
            methods::SendAudio::new(chat_id, InputFile::id(id))
                .reply_parameters_option(reply_to_message_id.map(|id| ReplyParameters::new(id).allow_sending_without_reply(true)))
                .disable_notification(true),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        info!("Audio sent");

        Ok(())
    }
}

pub struct EditVideo {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

impl Interactor<EditMediaInput<'_>> for &EditVideo {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        EditMediaInput {
            inline_message_id,
            id,
            caption,
        }: EditMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Video editing");
        send::with_retries(
            &self.bot,
            methods::EditMessageMedia::new(
                InputMediaVideo::new(InputFile::id(id))
                    .caption(caption)
                    .supports_streaming(true)
                    .parse_mode(ParseMode::HTML),
            )
            .inline_message_id(inline_message_id)
            .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        info!("Video edited");

        Ok(())
    }
}

pub struct EditAudio {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

impl Interactor<EditMediaInput<'_>> for &EditAudio {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        EditMediaInput {
            inline_message_id,
            id,
            caption,
        }: EditMediaInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Audio editing");
        send::with_retries(
            &self.bot,
            methods::EditMessageMedia::new(InputMediaAudio::new(InputFile::id(id)).caption(caption).parse_mode(ParseMode::HTML))
                .inline_message_id(inline_message_id)
                .reply_markup(InlineKeyboardMarkup::new([[]])),
            2,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        info!("Audio edited");

        Ok(())
    }
}

pub struct SendVideoPlaylist {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

impl Interactor<SendPlaylistInput> for &SendVideoPlaylist {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendPlaylistInput {
            chat_id,
            reply_to_message_id,
            playlist,
        }: SendPlaylistInput,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Video playlist sending");
        send::media_groups(
            &self.bot,
            chat_id,
            playlist
                .into_iter()
                .map(|val| InputMediaVideo::new(InputFile::id(val.file_id)))
                .collect(),
            reply_to_message_id,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        info!("Video playlist sent");

        Ok(())
    }
}

pub struct SendAudioPlaylist {
    pub bot: Arc<Bot>,
    pub timeouts_cfg: Arc<TimeoutsConfig>,
}

impl Interactor<SendPlaylistInput> for &SendAudioPlaylist {
    type Output = ();
    type Err = SessionErrorKind;

    #[instrument(skip_all)]
    async fn execute(
        self,
        SendPlaylistInput {
            chat_id,
            reply_to_message_id,
            playlist,
        }: SendPlaylistInput,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Audio playlist sending");
        send::media_groups(
            &self.bot,
            chat_id,
            playlist
                .into_iter()
                .map(|MediaInPlaylist { file_id, .. }| InputMediaAudio::new(InputFile::id(file_id)))
                .collect(),
            reply_to_message_id,
            Some(self.timeouts_cfg.send_by_id),
        )
        .await?;
        info!("Audio playlist sent");

        Ok(())
    }
}
