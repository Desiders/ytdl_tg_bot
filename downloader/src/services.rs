pub mod domain_replacer;
pub mod ffmpeg;
pub mod gallery_dl;
pub mod snapsave;
pub mod songrec;
pub mod spotdl;
pub mod thumbnail;
pub mod ytdl;

pub use domain_replacer::DomainReplacer;
pub use ffmpeg::download_and_convert;
pub use snapsave::SnapsaveResolver;
pub use songrec::SongRecognizer;
pub use spotdl::SpotdlResolver;
pub use thumbnail::embed_thumbnail;
