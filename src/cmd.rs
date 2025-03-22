pub mod ffmpeg;
pub mod ytdl;

pub use ffmpeg::{convert_to_jpg, merge_streams};
pub use ytdl::{download_audio_to_path, download_to_pipe, download_video_to_path, get_media_or_playlist_info};
