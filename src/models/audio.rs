use std::path::PathBuf;

#[derive(Debug)]
pub struct TgAudioInPlaylist {
    pub file_id: Box<str>,
    pub index_in_playlist: usize,
}

impl TgAudioInPlaylist {
    pub fn new(file_id: impl Into<Box<str>>, index_in_playlist: usize) -> Self {
        Self {
            file_id: file_id.into(),
            index_in_playlist,
        }
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub struct AudioInFS {
    pub path: PathBuf,
    pub thumbnail_path: Option<PathBuf>,
}

impl AudioInFS {
    pub fn new(path: impl Into<PathBuf>, thumbnail_path: Option<PathBuf>) -> Self {
        Self {
            path: path.into(),
            thumbnail_path,
        }
    }
}
