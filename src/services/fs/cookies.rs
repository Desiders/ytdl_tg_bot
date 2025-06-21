use crate::models::Cookies;

use std::{io, path::Path};
use tracing::{event, Level};
use url::Host;

pub fn get_cookies_from_directory(path: impl AsRef<Path>) -> Result<Cookies, io::Error> {
    let path = path.as_ref();
    let mut cookies = Cookies::default();

    if !path.exists() {
        event!(Level::WARN, "Cookies directory does not exist: {}", path.display());
        return Ok(cookies);
    }

    for entry in path.read_dir()? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            let path = entry.path();
            let host = if let Some(host) = path.file_stem() {
                let Ok(host) = Host::parse(host.to_string_lossy().as_ref()) else {
                    continue;
                };
                host
            } else {
                event!(Level::ERROR, "Invalid cookie file name: {}", path.display());
                continue;
            };
            if let Some(extension) = path.extension() {
                if extension != "txt" {
                    event!(Level::WARN, "Skipping non-txt cookie file: {host}");
                    continue;
                }
            } else {
                event!(Level::WARN, "Skipping file without extension: {host}");
                continue;
            }
            cookies.add_cookie(host, path);
        }
    }

    Ok(cookies)
}
