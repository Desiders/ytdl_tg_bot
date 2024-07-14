use std::{
    fs::{self, Metadata},
    io,
    path::{Path, PathBuf},
};
use tracing::instrument;

const MAX_THUMBNAIL_SIZE_IN_BYTES: u64 = 1024 * 200; // 200 KB
const ACCEPTABLE_THUMBNAIL_EXTENSIONS: [&str; 2] = ["jpg", "jpeg"];

#[instrument(skip_all, fields(path_dir = ?path_dir.as_ref()))]
pub fn get_best_thumbnail_path_in_dir(path_dir: impl AsRef<Path>) -> Result<Option<PathBuf>, io::Error> {
    let path_dir = path_dir.as_ref();

    let mut best_thumbnail: Option<(PathBuf, Metadata)> = None;

    for entry in fs::read_dir(path_dir)? {
        let entry = entry?;

        let path = entry.path();

        let Some(entry_extension) = path.extension() else {
            continue;
        };

        if !ACCEPTABLE_THUMBNAIL_EXTENSIONS.contains(&entry_extension.to_str().unwrap_or_default()) {
            continue;
        }

        let entry_metadata = entry.metadata()?;
        let entry_size = entry_metadata.len();

        if entry_size > MAX_THUMBNAIL_SIZE_IN_BYTES {
            continue;
        }

        if let Some((_, best_thumbnail_metadata)) = best_thumbnail.as_ref() {
            if entry_size > best_thumbnail_metadata.len() {
                best_thumbnail = Some((path, entry_metadata));
            }
        } else {
            best_thumbnail = Some((path, entry_metadata));
        }
    }

    Ok(best_thumbnail.map(|(path, _)| path))
}
