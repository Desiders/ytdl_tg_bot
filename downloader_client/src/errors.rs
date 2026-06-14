use std::io;

use crate::NodeFailoverError;

#[derive(Debug, thiserror::Error)]
pub enum GetMediaInfoErrorKind {
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error("All download nodes are busy. Try again later.")]
    NodeUnavailable,
    #[error(
        "The source site rejected this media (for example: login required, geo restriction, or temporary anti-bot limits). Try another URL or try again later."
    )]
    NodeContextUnavailable,
}

impl From<NodeFailoverError<GetMediaInfoErrorKind>> for GetMediaInfoErrorKind {
    fn from(err: NodeFailoverError<GetMediaInfoErrorKind>) -> Self {
        match err {
            NodeFailoverError::NodeUnavailable => Self::NodeUnavailable,
            NodeFailoverError::NodeContextUnavailable => Self::NodeContextUnavailable,
            NodeFailoverError::Operation(err) => err,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveSourceErrorKind {
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error("Invalid resolved url: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("All download nodes are busy. Try again later.")]
    NodeUnavailable,
    #[error("Could not resolve a DRM-free source for this link.")]
    NodeContextUnavailable,
}

impl From<NodeFailoverError<ResolveSourceErrorKind>> for ResolveSourceErrorKind {
    fn from(err: NodeFailoverError<ResolveSourceErrorKind>) -> Self {
        match err {
            NodeFailoverError::NodeUnavailable => Self::NodeUnavailable,
            NodeFailoverError::NodeContextUnavailable => Self::NodeContextUnavailable,
            NodeFailoverError::Operation(err) => err,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RecognizeSongErrorKind {
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error("All download nodes are busy. Try again later.")]
    NodeUnavailable,
    #[error("Could not recognize the song.")]
    NodeContextUnavailable,
}

impl RecognizeSongErrorKind {
    /// True when the node reported that no song matched the clip (a normal "not found"), as opposed
    /// to a transport/processing failure.
    #[must_use]
    pub fn is_no_match(&self) -> bool {
        matches!(self, Self::Rpc(status) if status.code() == tonic::Code::NotFound)
    }
}

impl From<NodeFailoverError<RecognizeSongErrorKind>> for RecognizeSongErrorKind {
    fn from(err: NodeFailoverError<RecognizeSongErrorKind>) -> Self {
        match err {
            NodeFailoverError::NodeUnavailable => Self::NodeUnavailable,
            NodeFailoverError::NodeContextUnavailable => Self::NodeContextUnavailable,
            NodeFailoverError::Operation(err) => err,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Rpc(#[from] tonic::Status),
    #[error(transparent)]
    Metadata(#[from] tonic::metadata::errors::InvalidMetadataValue),
    #[error("Invalid download stream")]
    InvalidStream,
    #[error("All download nodes are busy. Try again later.")]
    NodeUnavailable,
    #[error(
        "The source site rejected this download (for example: login required, geo restriction, or temporary anti-bot limits). Try another URL or try again later."
    )]
    NodeContextUnavailable,
}

impl From<NodeFailoverError<DownloadErrorKind>> for DownloadErrorKind {
    fn from(err: NodeFailoverError<DownloadErrorKind>) -> Self {
        match err {
            NodeFailoverError::NodeUnavailable => Self::NodeUnavailable,
            NodeFailoverError::NodeContextUnavailable => Self::NodeContextUnavailable,
            NodeFailoverError::Operation(err) => err,
        }
    }
}
