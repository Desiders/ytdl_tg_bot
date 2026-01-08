use std::path::Path;

pub fn sanitize_send_filename(path: &Path, name: &str) -> String {
    let actual_extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    if actual_extension.is_empty() {
        return name.to_string();
    }

    let base_name = if let Some(pos) = name.rfind('.') {
        let suffix = &name[pos + 1..];
        if !suffix.is_empty() && suffix.len() <= 8 && suffix.chars().all(|c| c.is_ascii_alphanumeric()) {
            &name[..pos]
        } else {
            name
        }
    } else {
        name
    };

    if base_name.is_empty() {
        format!("{}.{}", "file", actual_extension)
    } else {
        format!("{}.{}", base_name, actual_extension)
    }
}


#[cfg(test)]
mod tests {
    use super::sanitize_send_filename;
    use std::path::Path;

    #[test]
    fn preserves_same_multi_extension_tar_gz() {
        let path = Path::new("/tmp/archive.tar.gz");
        let name = "backup.tar.gz";
        assert_eq!(sanitize_send_filename(path, name), "backup.tar.gz");
    }

    #[test]
    fn preserves_same_multi_extension_tag_gz() {
        let path = Path::new("/tmp/archive.tag.gz");
        let name = "release.tag.gz";
        assert_eq!(sanitize_send_filename(path, name), "release.tag.gz");
    }

    #[test]
    fn replaces_different_extension_with_last_component_of_multi_ext() {
        let path = Path::new("/tmp/archive.tar.gz");
        let name = "video.mp4";
        assert_eq!(sanitize_send_filename(path, name), "video.gz");
    }

    #[test]
    fn appends_last_extension_when_name_has_no_extension() {
        let path = Path::new("/tmp/archive.tar.gz");
        let name = "backup";
        assert_eq!(sanitize_send_filename(path, name), "backup.gz");
    }

    #[test]
    fn handles_name_with_single_part_that_matches_inner_of_multi_extension() {
        let path = Path::new("/tmp/archive.tar.gz");
        let name = "backup.tar";
        assert_eq!(sanitize_send_filename(path, name), "backup.gz");
    }

    #[test]
    fn keeps_non_alnum_suffix_and_appends_last_ext() {
        let path = Path::new("/tmp/archive.tar.gz");
        let name = "song.fake-ext";
        assert_eq!(sanitize_send_filename(path, name), "song.fake-ext.gz");
    }

    #[test]
    fn preserves_case_and_handles_uppercase_extension() {
        let path = Path::new("/tmp/archive.TAR.GZ");
        let name = "release.TAR.GZ";
        assert_eq!(sanitize_send_filename(path, name), "release.TAR.GZ");
    }

    #[test]
    fn keeps_name_if_path_has_no_extension() {
        let path = Path::new("/tmp/video");
        let name = "My title.mp4";
        assert_eq!(sanitize_send_filename(path, name), "My title.mp4");
    }

    #[test]
    fn replaces_name_extension_with_actual() {
        let path = Path::new("/tmp/video.webm");
        let name = "My title.mp4";
        assert_eq!(sanitize_send_filename(path, name), "My title.webm");
    }

    #[test]
    fn appends_extension_when_name_has_no_extension() {
        let path = Path::new("/tmp/audio.mp3");
        let name = "Track";
        assert_eq!(sanitize_send_filename(path, name), "Track.mp3");
    }

    #[test]
    fn preserves_weird_suffix_and_appends_extension() {
        let path = Path::new("/tmp/audio.mp3");
        let name = "song.fake-ext";
        assert_eq!(sanitize_send_filename(path, name), "song.fake-ext.mp3");
    }

    #[test]
    fn strips_short_alnum_suffix_only() {
        let path = Path::new("/tmp/v.mp4");
        let name = "complex.name.mkv";
        assert_eq!(sanitize_send_filename(path, name), "complex.name.mp4");
    }

    #[test]
    fn handles_empty_basename() {
        let path = Path::new("/tmp/out.webm");
        let name = ".webm";
        assert_eq!(sanitize_send_filename(path, name), "file.webm");
    }
}
