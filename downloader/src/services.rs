pub mod ffmpeg;
pub mod mutagen;
pub mod ytdl;

pub use ffmpeg::download_and_convert;
pub use mutagen::embed_thumbnail;
