use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum ErrorKind {
    #[error("Invalid URL format")]
    InvalidUrl,

    #[error("No video ID found in: {0}")]
    NoVideoIdFound(String),
}

const VIDEO_ID_LENGTH: usize = 11;

pub fn get_video_id(input: &str) -> Result<String, ErrorKind> {
    let url = Url::parse(input).map_err(|_| ErrorKind::InvalidUrl)?;
    let host = url.host_str().ok_or(ErrorKind::InvalidUrl)?;

    if host.ends_with("youtube.com") {
        if let Some(query_pairs) = url.query_pairs().find(|(key, _)| key == "v") {
            let id = query_pairs.1.to_string();
            if id.len() == VIDEO_ID_LENGTH {
                return Ok(id);
            }
        }

        let path_segments: Vec<&str> = url.path_segments().map(|c| c.collect()).unwrap_or_default();
        if path_segments.len() > 1 {
            match path_segments[0] {
                "embed" | "v" | "shorts" if path_segments[1].len() == VIDEO_ID_LENGTH => return Ok(path_segments[1].to_string()),
                "clip" => return Ok(path_segments[1].to_string()),
                _ => {}
            }
        }
    } else if host == "youtu.be" {
        let path_segments: Vec<&str> = url.path_segments().map(|c| c.collect()).unwrap_or_default();
        if let Some(id) = path_segments.first() {
            if id.len() == VIDEO_ID_LENGTH {
                return Ok(id.to_string());
            }
        }
    }

    Err(ErrorKind::NoVideoIdFound(input.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_url_standard() {
        assert_eq!(get_video_id("https://www.youtube.com/watch?v=abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_with_params() {
        assert_eq!(
            get_video_id("https://www.youtube.com/watch?v=abcdefghijk&si=someparam").unwrap(),
            "abcdefghijk"
        );
    }

    #[test]
    fn test_valid_url_youtu_be_with_params() {
        assert_eq!(get_video_id("https://youtu.be/abcdefghijk?si=someparam").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_mobile() {
        assert_eq!(get_video_id("https://m.youtube.com/watch?v=abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_embed() {
        assert_eq!(get_video_id("https://www.youtube.com/embed/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_shorts() {
        assert_eq!(get_video_id("https://www.youtube.com/shorts/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_clips() {
        assert_eq!(
            get_video_id("https://youtube.com/clip/UgkxAunuGm-TAKLTuefe7EpYiyFPLfTWR28D?si=pC4WroQ7YCmPm3wO").unwrap(),
            "UgkxAunuGm-TAKLTuefe7EpYiyFPLfTWR28D"
        );
    }

    #[test]
    fn test_valid_url_youtu_be() {
        assert_eq!(get_video_id("https://youtu.be/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_invalid_url_example() {
        assert!(get_video_id("https://www.example.com/watch?v=abcdefghijk").is_err());
    }

    #[test]
    fn test_invalid_url_non_video_path() {
        assert!(get_video_id("https://youtube.com/someotherpath").is_err());
    }

    #[test]
    fn test_invalid_url_short_id() {
        assert!(get_video_id("https://youtu.be/shortvideo").is_err());
    }

    #[test]
    fn test_invalid_url_not_a_url() {
        assert!(get_video_id("not a url").is_err());
    }

    #[test]
    fn test_invalid_url_homepage() {
        assert!(get_video_id("https://www.youtube.com/").is_err());
    }
}
