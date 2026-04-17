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
