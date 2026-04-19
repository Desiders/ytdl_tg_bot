#![allow(
    clippy::default_trait_access,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::too_many_lines
)]

pub mod downloader {
    #[allow(clippy::all)]
    mod generated {
        tonic::include_proto!("downloader");
    }

    pub use generated::*;
}
