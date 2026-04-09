mod auth;
mod client;
mod download;
mod errors;
mod handle;
mod media_info;
mod retry;
mod router;
mod selection;

pub use auth::authenticated_request;
pub use client::{DownloaderServiceTarget, DownloaderTlsConfig};
pub use download::{download_media, DownloadEvent, DownloadSession};
pub use errors::{DownloadErrorKind, GetMediaInfoErrorKind};
pub use handle::{NodeHandle, NodeHandleError};
pub use media_info::get_media_info;
pub use retry::{with_node_failover, NodeAttemptErrorKind, NodeFailoverError};
pub use router::{DownloaderClusterConfig, NodeRouter};
