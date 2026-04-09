mod auth;
mod client;
mod errors;
mod handle;
mod retry;
mod router;
mod selection;

pub use auth::authenticated_request;
pub use client::{DownloaderServiceTarget, DownloaderTlsConfig};
pub use errors::{DownloadErrorKind, GetMediaInfoErrorKind};
pub use handle::{NodeHandle, NodeHandleError};
pub use retry::{with_node_failover, NodeAttemptErrorKind, NodeFailoverError};
pub use router::{DownloaderClusterConfig, NodeRouter};
