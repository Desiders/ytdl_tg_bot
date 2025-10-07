use tempfile::TempDir;

use super::{format, PreferredLanguages, Video};
use crate::errors::FormatNotFound;

use std::path::PathBuf;

pub struct AudioAndFormat<'a> {
    pub video: &'a Video,
    pub format: format::Audio<'a>,
}

impl<'a> AudioAndFormat<'a> {
    pub fn new_with_select_format(
        video: &'a Video,
        max_file_size: u32,
        PreferredLanguages { languages }: &PreferredLanguages,
    ) -> Result<Self, FormatNotFound> {
        let mut formats = video.get_audio_formats();
        formats.sort(max_file_size, &languages);

        let Some(format) = formats.first().cloned() else {
            return Err(FormatNotFound);
        };

        Ok(Self { video, format })
    }
}

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

#[allow(clippy::module_name_repetitions)]
#[derive(Debug)]
pub struct AudioInFS {
    pub path: PathBuf,
    pub thumbnail_path: Option<PathBuf>,
    pub temp_dir: TempDir,
}

impl AudioInFS {
    pub fn new(path: impl Into<PathBuf>, thumbnail_path: Option<PathBuf>, temp_dir: TempDir) -> Self {
        Self {
            path: path.into(),
            thumbnail_path,
            temp_dir,
        }
    }
}
