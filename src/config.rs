use std::{
    borrow::Cow,
    env::{self, VarError},
    num::ParseIntError,
    str::ParseBoolError,
};

#[derive(Clone, Debug)]
pub struct Bot {
    pub token: String,
    pub source_code_url: String,
    pub receiver_video_chat_id: i64,
    pub telegram_bot_api_url: String,
}

#[derive(Clone, Debug)]
pub struct YtDlp {
    pub full_path: String,
    pub max_file_size: u32,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bot: Bot,
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
            telegram_bot_api_url: env::var("TELEGRAM_BOT_API_URL").map_err(|err| ErrorKind::Env {
                source: err,
                key: "TELEGRAM_BOT_API_URL".into(),
            })?,
        },
        yt_dlp: YtDlp {
            full_path: env::var("YT_DLP_FULL_PATH").map_err(|err| ErrorKind::Env {
                source: err,
                key: "YT_DLP_FULL_PATH".into(),
            })?,
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
