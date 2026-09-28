#![allow(
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::too_many_lines
)]

/// Hard total budget for one admitted media execution, including streaming.
pub const MEDIA_EXECUTION_LIMIT_SECS: u64 = 360;
/// Set only on a downloader stream error after its execution future has ended.
pub const TERMINAL_DOWNLOAD_STATUS_HEADER: &str = "x-download-terminal";

pub mod downloader {
    #[allow(clippy::all)]
    mod generated {
        tonic::include_proto!("downloader");
    }

    pub use generated::*;
}
