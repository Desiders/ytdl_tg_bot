use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct CookieRecord {
    pub cookie_id: String,
    pub domain: String,
    pub path: PathBuf,
}

pub fn load_cookies_from_directory(root: &Path) -> Result<Vec<CookieRecord>, io::Error> {
    let mut cookies = Vec::new();

    if !root.exists() {
        return Ok(cookies);
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            let domain = entry.file_name().to_string_lossy().to_string();
            if !domain.starts_with('.') && !domain.contains("__") {
                cookies.extend(load_domain_cookies(&path, &domain)?);
            }
            continue;
        }
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "txt") {
            continue;
        }
        let Some(encoded_cookie_name) = path.file_name().map(|name| name.to_string_lossy().to_string()) else {
            continue;
        };
        let Some((domain, file_name)) = decode_cookie_entry_name(&encoded_cookie_name) else {
            continue;
        };

        cookies.push(CookieRecord {
            cookie_id: format!("{domain}/{file_name}"),
            domain,
            path,
        });
    }

    cookies.sort_by(|a, b| a.cookie_id.cmp(&b.cookie_id));
    Ok(cookies)
}

fn load_domain_cookies(root: &Path, domain: &str) -> Result<Vec<CookieRecord>, io::Error> {
    let mut cookies = Vec::new();

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "txt") {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') || file_name.contains("__") {
            continue;
        }

        cookies.push(CookieRecord {
            cookie_id: format!("{domain}/{file_name}"),
            domain: domain.to_owned(),
            path,
        });
    }

    Ok(cookies)
}

fn decode_cookie_entry_name(encoded_cookie_name: &str) -> Option<(String, String)> {
    let (domain, file_name) = encoded_cookie_name.split_once("__")?;
    if domain.is_empty() || file_name.is_empty() {
        return None;
    }
    Some((domain.to_owned(), file_name.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::{
        env::temp_dir,
        process::id,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
            let path = temp_dir().join(format!(
                "cookie-assignment-test-{}-{}",
                id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn nested_and_flat_cookies_preserve_ids_domains_and_order() {
        let directory = TestDirectory::new();
        let nested = directory.0.join("nested");
        let flat = directory.0.join("flat");
        fs::create_dir(&nested).unwrap();
        fs::create_dir(&flat).unwrap();
        for (domain, file_name) in [
            ("b.example.test", "main.txt"),
            ("a.example.test", "main.txt"),
            ("b.example.test", "extra.txt"),
        ] {
            fs::create_dir_all(nested.join(domain)).unwrap();
            let content = format!("{domain}/{file_name}");
            fs::write(nested.join(domain).join(file_name), &content).unwrap();
            fs::write(flat.join(format!("{domain}__{file_name}")), content).unwrap();
        }

        let nested_cookies = load_cookies_from_directory(&nested).unwrap();
        let flat_cookies = load_cookies_from_directory(&flat).unwrap();
        assert_eq!(
            nested_cookies.iter().map(|cookie| cookie.cookie_id.as_str()).collect::<Vec<_>>(),
            ["a.example.test/main.txt", "b.example.test/extra.txt", "b.example.test/main.txt"]
        );
        assert_eq!(nested_cookies.len(), flat_cookies.len());
        for (nested_cookie, flat_cookie) in nested_cookies.iter().zip(&flat_cookies) {
            assert_eq!(nested_cookie.cookie_id, flat_cookie.cookie_id);
            assert_eq!(nested_cookie.domain, flat_cookie.domain);
            assert_eq!(fs::read(&nested_cookie.path).unwrap(), fs::read(&flat_cookie.path).unwrap());
        }
    }

    #[cfg(unix)]
    #[test]
    fn secret_symlinks_load_once_and_unsupported_nested_paths_are_ignored() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new();
        let root = &directory.0;
        let projected = root.join("..20261014");
        fs::create_dir(&projected).unwrap();
        fs::write(projected.join("b.example.test__main.txt"), "cookie").unwrap();
        symlink("..20261014", root.join("..data")).unwrap();
        symlink("..data/b.example.test__main.txt", root.join("b.example.test__main.txt")).unwrap();
        fs::create_dir_all(root.join("a.example.test/deep.txt")).unwrap();
        fs::write(root.join("a.example.test/deep.txt/main.txt"), "nested").unwrap();
        fs::write(root.join("a.example.test/reserved__name.txt"), "reserved").unwrap();
        fs::write(root.join("a.example.test/ignored.bin"), "other").unwrap();
        fs::create_dir(root.join("reserved__domain")).unwrap();
        fs::write(root.join("reserved__domain/main.txt"), "reserved").unwrap();
        fs::write(root.join("plain.txt"), "other").unwrap();

        let cookies = load_cookies_from_directory(root).unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].cookie_id, "b.example.test/main.txt");
        assert_eq!(cookies[0].domain, "b.example.test");
        assert_eq!(cookies[0].path, root.join("b.example.test__main.txt"));
        assert_eq!(fs::read_to_string(&cookies[0].path).unwrap(), "cookie");
    }
}
