use std::{
    fs::Metadata,
    io,
    path::{Path, PathBuf},
};
use tracing::{event, Level};

const MAX_THUMBNAIL_SIZE_IN_BYTES: u64 = 1024 * 200; // 200 KB
const ACCEPTABLE_THUMBNAIL_EXTENSIONS: [&str; 2] = ["jpg", "jpeg"];

pub async fn get_best_thumbnail_path_in_dir(path_dir: impl AsRef<Path>, name: &str) -> Result<Option<PathBuf>, io::Error> {
    let path_dir = path_dir.as_ref();

    let mut read_dir = tokio::fs::read_dir(path_dir).await?;

    let mut best_thumbnail: Option<(PathBuf, Metadata)> = None;

    while let Some(entry) = read_dir.next_entry().await? {
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

        let entry_metadata = entry.metadata().await?;
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

            best_thumbnail = Some((entry.path(), entry.metadata().await?));
        }
    }

    Ok(best_thumbnail.map(|(path, _)| path))
}
