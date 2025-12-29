use crate::{
    entities::{AudioInFS, VideoInFS},
    handlers_utils::send,
    interactors::Interactor,
};

use std::path::Path;
use std::sync::Arc;
use telers::{
    errors::SessionErrorKind,
    methods::{DeleteMessage, SendAudio, SendVideo},
    types::{InputFile, ReplyParameters},
    Bot,
};
use tracing::{debug, error, info, instrument};

const SEND_TIMEOUT: f32 = 180.0;

pub struct SendVideoInFS {
    bot: Arc<Bot>,
}

impl SendVideoInFS {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
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
}

impl SendAudioInFS {
    pub const fn new(bot: Arc<Bot>) -> Self {
        Self { bot }
    }
}

pub struct SendAudioInFSInput<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub audio_in_fs: AudioInFS,
    pub name: &'a str,
    pub performer: Option<&'a str>,
    pub title: Option<&'a str>,
    pub duration: Option<i64>,
    pub with_delete: bool,
}

impl<'a> SendAudioInFSInput<'a> {
    pub const fn new(
        chat_id: i64,
        reply_to_message_id: Option<i64>,
        audio_in_fs: AudioInFS,
        name: &'a str,
        performer: Option<&'a str>,
        title: Option<&'a str>,
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

fn sanitize_send_filename(path: &Path, name: &str) -> String {
    let actual_extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    if actual_extension.is_empty() {
        return name.to_string();
    }

    let base_name = if let Some(pos) = name.rfind('.') {
        let suffix = &name[pos + 1..];
        if !suffix.is_empty() && suffix.len() <= 8 && suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
            &name[..pos]
        } else {
            name
        }
    } else {
        name
    };

    if base_name.is_empty() {
        format!("{}.{}", "file", actual_extension)
    } else {
        format!("{}.{}", base_name, actual_extension)
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
            Some(SEND_TIMEOUT),
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

    #[instrument(skip_all, fields(name, duration, with_delete))]
    async fn execute(
        self,
        SendAudioInFSInput {
            chat_id,
            reply_to_message_id,
            audio_in_fs: AudioInFS { path, temp_dir, .. },
            name,
            performer,
            title,
            duration,
            with_delete,
        }: SendAudioInFSInput<'_>,
    ) -> Result<Self::Output, Self::Err> {
        debug!("Audio sending");

        let send_name = sanitize_send_filename(path.as_ref(), name);

        let mut method = SendAudio::new(chat_id, InputFile::fs_with_name(path, &send_name))
            .disable_notification(true)
            .duration_option(duration);

        if let Some(p) = performer {
            method = method.performer_option(Some(p.to_string()));
        }
        if let Some(t) = title {
            method = method.title_option(Some(t.to_string()));
        }
        if let Some(reply_to) = reply_to_message_id {
            method = method.reply_parameters_option(Some(ReplyParameters::new(reply_to).allow_sending_without_reply(true)));
        }

        let message = send::with_retries(&self.bot, method, 2, Some(SEND_TIMEOUT)).await?;
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

#[cfg(test)]
mod tests {
    use super::sanitize_send_filename;
    use std::path::Path;

    #[test]
    fn keeps_name_if_path_has_no_extension() {
        let path = Path::new("/tmp/video");
        let name = "My title.mp4";
        assert_eq!(sanitize_send_filename(path, name), "My title.mp4");
    }

    #[test]
    fn replaces_name_extension_with_actual() {
        let path = Path::new("/tmp/video.webm");
        let name = "My title.mp4";
        assert_eq!(sanitize_send_filename(path, name), "My title.webm");
    }

    #[test]
    fn appends_extension_when_name_has_no_extension() {
        let path = Path::new("/tmp/audio.mp3");
        let name = "Track";
        assert_eq!(sanitize_send_filename(path, name), "Track.mp3");
    }

    #[test]
    fn preserves_weird_suffix_and_appends_extension() {
        let path = Path::new("/tmp/audio.mp3");
        let name = "song.fake-ext";
        assert_eq!(sanitize_send_filename(path, name), "song.fake-ext.mp3");
    }

    #[test]
    fn strips_short_alnum_suffix_only() {
        let path = Path::new("/tmp/v.mp4");
        let name = "complex.name.mkv";
        assert_eq!(sanitize_send_filename(path, name), "complex.name.mp4");
    }

    #[test]
    fn handles_empty_basename() {
        let path = Path::new("/tmp/out.webm");
        let name = ".webm";
        assert_eq!(sanitize_send_filename(path, name), "file.webm");
    }
}
