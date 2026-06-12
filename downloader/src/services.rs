pub mod domain_replacer;
pub mod ffmpeg;
pub mod gallery_dl;
pub mod mutagen;
pub mod spotdl;
pub mod ytdl;

pub use domain_replacer::DomainReplacer;
pub use ffmpeg::download_and_convert;
pub use mutagen::embed_thumbnail;
pub use spotdl::SpotdlResolver;
