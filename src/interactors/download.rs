pub mod audio;
pub mod video;

pub use audio::{
    DownloadAudio, DownloadAudioInput, DownloadAudioOutput, DownloadAudioPlaylist, DownloadAudioPlaylistInput, DownloadAudioPlaylistOutput,
};
pub use video::{
    DownloadVideo, DownloadVideoInput, DownloadVideoOutput, DownloadVideoPlaylist, DownloadVideoPlaylistInput, DownloadVideoPlaylistOutput,
};
