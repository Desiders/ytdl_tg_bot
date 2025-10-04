use std::path::PathBuf;

#[derive(Debug)]
pub struct TgAudioInPlaylist {
    pub file_id: Box<str>,
    pub index: usize,
}

impl TgAudioInPlaylist {
    pub fn new(file_id: impl Into<Box<str>>, index: usize) -> Self {
        Self {
            file_id: file_id.into(),
            index,
        }
    }
}

#[derive(Debug)]
pub struct TgAudio {
    pub file_id: Box<str>,
}

impl TgAudio {
    pub fn new(file_id: impl Into<Box<str>>) -> Self {
        Self { file_id: file_id.into() }
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
