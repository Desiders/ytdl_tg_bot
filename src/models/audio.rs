use std::path::PathBuf;

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
