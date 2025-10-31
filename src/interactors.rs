mod base;
mod chat;
mod downloaded_media;
mod get_media_info;

pub mod download;
pub mod send_media;

pub use base::Interactor;
pub use chat::{SaveChat, SaveChatInput};
pub use downloaded_media::{AddDownloadedAudio, AddDownloadedMediaInput, AddDownloadedVideo};
pub use get_media_info::{
    GetAudioByURL, GetAudioByURLInput, GetAudioByURLKind, GetMediaByURLErrorKind, GetMediaInfoById, GetMediaInfoByIdInput,
    GetShortMediaByURLInfo, GetShortMediaInfoByURLInput, GetUncachedVideoByURL, GetUncachedVideoByURLInput, GetUncachedVideoByURLKind,
    GetVideoByURL, GetVideoByURLInput, GetVideoByURLKind, SearchMediaInfo, SearchMediaInfoInput,
};
