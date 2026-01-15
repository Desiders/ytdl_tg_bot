pub fn parse_params(param_block: &str) -> Vec<(Box<str>, Box<str>)> {
    let inner = param_block.trim();
    // Remove the surrounding brackets if they exist.
    let inner = inner.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(inner);
    let params = inner
        .split(',')
        .filter_map(|param| {
            let trimmed = param.trim();
            if trimmed.is_empty() {
                None
            } else {
                let parts: Vec<&str> = trimmed.splitn(2, '=').collect();
                if parts.len() == 2 {
                    let key = parts[0].trim();
                    let value = parts[1].trim();
                    if key.is_empty() || value.is_empty() {
                        None
                    } else {
                        Some((key.to_owned().into_boxed_str(), value.to_owned().into_boxed_str()))
                    }
                } else {
                    None
                }
            }
        })
        .collect::<Vec<_>>();
    params
}
