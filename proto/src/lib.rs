#![allow(
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::too_many_lines
)]

/// Hard total budget for one admitted media execution, including streaming.
pub const MEDIA_EXECUTION_LIMIT_SECS: u64 = 360;
/// A selected native format disappeared before execution started.
pub const VIDEO_FORMAT_UNAVAILABLE: &str = "Requested video format is unavailable";
/// Set only on a downloader stream error after its execution future has ended.
pub const TERMINAL_DOWNLOAD_STATUS_HEADER: &str = "x-download-terminal";
/// The only pre-admission `RESOURCE_EXHAUSTED` response that permits node failover.
pub const NODE_CAPACITY_REJECTION_MESSAGE: &str = "Node is at capacity";

pub mod downloader {
    #[allow(clippy::all)]
    mod generated {
        tonic::include_proto!("downloader");
    }

    pub use generated::*;
}
