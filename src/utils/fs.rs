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