use crate::entities::Cookies;

use std::{io, path::Path};
use tracing::{debug, error, instrument, warn};
use url::Host;

#[instrument(skip_all)]
pub fn get_cookies_from_directory(path: impl AsRef<Path>) -> Result<Cookies, io::Error> {
    let path = path.as_ref();
    let mut cookies = Cookies::default();

    if !path.exists() {
        warn!("Cookies directory does not exist: {}", path.display());
        return Ok(cookies);
    }

    for entry in path.read_dir()? {
        let entry = entry?;

        let file_type = entry.file_type()?;
        if !file_type.is_file() && !file_type.is_symlink() {
            debug!("Skipping non-file entry: {}", entry.path().display());
            continue;
        }

        let path = entry.path();
        let host = if let Some(host) = path.file_stem() {
            let Ok(host) = Host::parse(host.to_string_lossy().as_ref()) else {
                continue;
            };
            host
        } else {
            error!("Invalid cookie file name: {}", path.display());
            continue;
        };
        if let Some(extension) = path.extension() {
            if extension != "txt" {
                warn!("Skipping non-txt cookie file: {host}");
                continue;
            }
        } else {
            warn!("Skipping file without extension: {host}");
            continue;
        }
        cookies.add_cookie(host, path);
    }

    Ok(cookies)
}
