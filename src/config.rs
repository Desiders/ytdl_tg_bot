use std::{
    borrow::Cow,
    env::{self, VarError},
    num::ParseIntError,
    str::ParseBoolError,
};

#[derive(Clone, Debug)]
pub struct Bot {
    pub token: String,
}

#[derive(Clone, Debug)]
pub struct YtDlp {
    pub dir_path: String,
    pub full_path: String,
    pub update_on_startup: bool,
    pub remove_on_shutdown: bool,
    pub max_files_size_in_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bot: Bot,
    pub yt_dlp: YtDlp,
}

#[derive(thiserror::Error, Debug)]
pub enum ErrorKind {
    #[error("env error: {source} for key {key}")]
    Env {
        source: VarError,
        key: Cow<'static, str>,
    },
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
        },
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
            max_files_size_in_bytes: env::var("YT_DLP_MAX_FILES_SIZE_IN_BYTES")
                .map_err(|err| ErrorKind::Env {
                    source: err,
                    key: "YT_DLP_MAX_FILES_SIZE_IN_BYTES".into(),
                })?
                .parse()
                .map_err(ErrorKind::ParseInt)?,
        },
    })
}
