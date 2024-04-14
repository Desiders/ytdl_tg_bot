use std::{
    fs::{self, Metadata},
    io,
    path::{Path, PathBuf},
};
use tracing::{event, instrument, Level};

const MAX_THUMBNAIL_SIZE_IN_BYTES: u64 = 1024 * 200; // 200 KB
const ACCEPTABLE_THUMBNAIL_EXTENSIONS: [&str; 2] = ["jpg", "jpeg"];

#[instrument(skip_all, fields(path_dir = ?path_dir.as_ref(), name = %name.as_ref()))]
pub fn get_best_thumbnail_path_in_dir(path_dir: impl AsRef<Path>, name: impl AsRef<str>) -> Result<Option<PathBuf>, io::Error> {
    let path_dir = path_dir.as_ref();
    let name = name.as_ref();

    let mut best_thumbnail: Option<(PathBuf, Metadata)> = None;

    for entry in fs::read_dir(path_dir)? {
        let entry = entry?;

        let entry_name = entry.file_name();

        // If names are equal, then it's video file, not thumbnail
        if entry_name == name {
            continue;
        }

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
                event!(Level::TRACE, path = ?entry.path(), "Got better thumbnail");

                best_thumbnail = Some((path, entry_metadata));
            }
        } else {
            event!(Level::TRACE, path = ?entry.path(), "Got first thumbnail");

            best_thumbnail = Some((path, entry_metadata));
        }
    }

    event!(Level::TRACE, "Best thumbnail: {best_thumbnail:?}");

    Ok(best_thumbnail.map(|(path, _)| path))
}
