use regex::Regex;

#[derive(thiserror::Error, Debug)]
pub enum ErrorKind {
    #[error("No video id found: {0}")]
    NoVideoIdFound(String),
}

lazy_static::lazy_static! {
    static ref YOUTUBE_REGEX: Regex = Regex::new(
        r"^(?:https?://)?(?:www\.|m\.|music\.|gaming\.)?youtube\.com/(?:watch\?v=|embed/|v/|shorts/)([a-zA-Z0-9_-]{11})|^(?:https?://)?youtu\.be/([a-zA-Z0-9_-]{11})"
    ).unwrap();
}

pub fn get_video_id(input: &str) -> Result<String, ErrorKind> {
    let input = input.trim();

    if let Some(caps) = YOUTUBE_REGEX.captures(input) {
        if let Some(id) = caps.get(1).or(caps.get(2)) {
            return Ok(id.as_str().to_string());
        }
    }

    Err(ErrorKind::NoVideoIdFound(input.to_string()))
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
        assert_eq!(
            get_video_id("https://youtu.be/HAIDqt2aUek?si=ZTzATAeAuShqm9j").unwrap(),
            "HAIDqt2aUek"
        );
    }

    #[test]
    fn test_valid_url_mobile() {
        assert_eq!(get_video_id("https://m.youtube.com/watch?v=abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_music() {
        assert_eq!(
            get_video_id("https://music.youtube.com/watch?v=abcdefghijk").unwrap(),
            "abcdefghijk"
        );
    }

    #[test]
    fn test_valid_url_gaming() {
        assert_eq!(
            get_video_id("https://gaming.youtube.com/watch?v=abcdefghijk").unwrap(),
            "abcdefghijk"
        );
    }

    #[test]
    fn test_valid_url_embed() {
        assert_eq!(get_video_id("https://www.youtube.com/embed/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_v() {
        assert_eq!(get_video_id("https://www.youtube.com/v/abcdefghijk").unwrap(), "abcdefghijk");
    }

    #[test]
    fn test_valid_url_shorts() {
        assert_eq!(get_video_id("https://www.youtube.com/shorts/abcdefghijk").unwrap(), "abcdefghijk");
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

    #[test]
    fn test_invalid_url_too_long_id() {
        assert!(get_video_id("abcdefghijklmno").is_err());
    }
}
