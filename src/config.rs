use std::{
    borrow::Cow,
    env::{self, VarError},
    num::ParseIntError,
    ops::Deref,
    str::ParseBoolError,
};
use telers::extractors::FromContext;

#[derive(Clone, Debug)]
pub struct Bot {
    pub token: String,
    pub source_code_url: String,
    pub receiver_video_chat_id: i64,
}

#[derive(Clone, Debug, FromContext)]
#[context(key = "phantom_video_id")]
pub struct PhantomVideoId(pub String);

impl Deref for PhantomVideoId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug, FromContext)]
#[context(key = "phantom_audio_id")]
pub struct PhantomAudioId(pub String);

impl Deref for PhantomAudioId {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub enum PhantomVideo {
    Id(PhantomVideoId),
    Path(String),
}

#[derive(Clone, Debug)]
pub enum PhantomAudio {
    Id(PhantomAudioId),
    Path(String),
}

#[derive(Clone, Debug)]
pub struct YtDlp {
    pub dir_path: String,
    pub full_path: String,
    pub update_on_startup: bool,
    pub remove_on_shutdown: bool,
    pub max_file_size: u64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bot: Bot,
    pub phantom_video: PhantomVideo,
    pub phantom_audio: PhantomAudio,
    pub yt_dlp: YtDlp,
}

#[derive(thiserror::Error, Debug)]
pub enum ErrorKind {
    #[error("env error: {source} for key {key}")]
    Env { source: VarError, key: Cow<'static, str> },
    #[error(transparent)]
    ParseInt(#[from] ParseIntError),
    #[error(transparent)]
    ParseBool(#[from] ParseBoolError),
}

pub fn read_config_from_env() -> Result<Config, ErrorKind> {
    let phantom_video_id_env = env::var("PHANTOM_VIDEO_ID");
    let phantom_video_path = env::var("PHANTOM_VIDEO_PATH");

    let phantom_video = if phantom_video_id_env.is_ok() && !phantom_video_id_env.as_ref().unwrap().is_empty() {
        #[allow(clippy::unnecessary_unwrap)]
        PhantomVideo::Id(PhantomVideoId(phantom_video_id_env.unwrap()))
    } else if phantom_video_path.is_ok() && !phantom_video_path.as_ref().unwrap().is_empty() {
        #[allow(clippy::unnecessary_unwrap)]
        PhantomVideo::Path(phantom_video_path.unwrap())
    } else {
        return Err(ErrorKind::Env {
            source: VarError::NotPresent,
            key: "PHANTOM_VIDEO_ID or PHANTOM_VIDEO_PATH".into(),
        });
    };

    let phantom_audio_id_env = env::var("PHANTOM_AUDIO_ID");
    let phantom_audio_path = env::var("PHANTOM_AUDIO_PATH");

    let phantom_audio = if phantom_audio_id_env.is_ok() && !phantom_audio_id_env.as_ref().unwrap().is_empty() {
        #[allow(clippy::unnecessary_unwrap)]
        PhantomAudio::Id(PhantomAudioId(phantom_audio_id_env.unwrap()))
    } else if phantom_audio_path.is_ok() && !phantom_audio_path.as_ref().unwrap().is_empty() {
        #[allow(clippy::unnecessary_unwrap)]
        PhantomAudio::Path(phantom_audio_path.unwrap())
    } else {
        return Err(ErrorKind::Env {
            source: VarError::NotPresent,
            key: "PHANTOM_AUDIO_ID or PHANTOM_AUDIO_PATH".into(),
        });
    };

    Ok(Config {
        bot: Bot {
            token: env::var("BOT_TOKEN").map_err(|err| ErrorKind::Env {
                source: err,
                key: "BOT_TOKEN".into(),
            })?,
            source_code_url: env::var("BOT_SOURCE_CODE_URL").map_err(|err| ErrorKind::Env {
                source: err,
                key: "BOT_SOURCE_CODE_URL".into(),
            })?,
            receiver_video_chat_id: env::var("RECEIVER_VIDEO_CHAT_ID")
                .map_err(|err| ErrorKind::Env {
                    source: err,
                    key: "RECEIVER_VIDEO_CHAT_ID".into(),
                })?
                .parse()
                .map_err(ErrorKind::ParseInt)?,
        },
        phantom_video,
        phantom_audio,
        yt_dlp: YtDlp {
            dir_path: env::var("YT_DLP_DIR_PATH").map_err(|err| ErrorKind::Env {
                source: err,
                key: "YT_DLP_PATH".into(),
            })?,
            full_path: env::var("YT_DLP_FULL_PATH").map_err(|err| ErrorKind::Env {
                source: err,
                key: "YT_DLP_FULL_PATH".into(),
            })?,
            update_on_startup: env::var("YT_DLP_UPDATE_ON_STARTUP")
                .map_err(|err| ErrorKind::Env {
                    source: err,
                    key: "YT_DLP_UPDATE_ON_STARTUP".into(),
                })?
                .parse()
                .map_err(ErrorKind::ParseBool)?,
            remove_on_shutdown: env::var("YT_DLP_REMOVE_ON_SHUTDOWN")
                .map_err(|err| ErrorKind::Env {
                    source: err,
                    key: "YT_DLP_REMOVE_ON_SHUTDOWN".into(),
                })?
                .parse()
                .map_err(ErrorKind::ParseBool)?,
            max_file_size: env::var("YT_DLP_MAX_FILE_SIZE")
                .map_err(|err| ErrorKind::Env {
                    source: err,
                    key: "YT_DLP_MAX_FILE_SIZE".into(),
                })?
                .parse()
                .map_err(ErrorKind::ParseInt)?,
        },
    })
}
