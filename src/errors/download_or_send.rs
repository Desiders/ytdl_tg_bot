use super::DownloadError;

use telers::errors::SessionErrorKind;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Download error: {0}")]
    Download(#[from] DownloadError),
    #[error("Send error: {0}")]
    Send(#[from] SessionErrorKind),
}
