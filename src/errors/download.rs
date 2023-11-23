use crate::cmd::ytdl;

use std::io;

#[derive(thiserror::Error, Debug)]
#[error("Download error: {0}")]
pub enum Error {
    #[error("No format found for video {video_id}")]
    NoFormatFound { video_id: Box<str> },
    #[error("Failed to download video: {0}")]
    DownloadFailed(#[from] ytdl::Error),
    #[error("Failed to get best thumbnail path in dir: {0}")]
    ThumbnailPathFailed(io::Error),
}
