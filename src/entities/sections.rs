use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ParseSectionError {
    #[error("Invalid time format")]
    InvalidTime,
    #[error("Invalid section format. Expected \"start-end\".")]
    InvalidFormat,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Sections {
    pub start: Option<i32>,
    pub end: Option<i32>,
}

impl Sections {
    fn parse_section(raw: &str) -> Result<Option<i32>, ParseSectionError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }

        let parts: Vec<&str> = raw.split(':').collect();

        let secs = match parts.len() {
            1 => parts[0].parse::<i32>().map_err(|_| ParseSectionError::InvalidTime)?,
            2 => {
                let m = parts[0].parse::<i32>().map_err(|_| ParseSectionError::InvalidTime)?;
                let s = parts[1].parse::<i32>().map_err(|_| ParseSectionError::InvalidTime)?;
                m * 60 + s
            }
            3 => {
                let h = parts[0].parse::<i32>().map_err(|_| ParseSectionError::InvalidTime)?;
                let m = parts[1].parse::<i32>().map_err(|_| ParseSectionError::InvalidTime)?;
                let s = parts[2].parse::<i32>().map_err(|_| ParseSectionError::InvalidTime)?;
                h * 3600 + m * 60 + s
            }
            _ => return Err(ParseSectionError::InvalidTime),
        };

        Ok(Some(secs))
    }

    fn format_time(t: i32) -> String {
        let h = t / 3600;
        let m = (t % 3600) / 60;
        let s = t % 60;

        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else if m > 0 {
            format!("{m}:{s:02}")
        } else {
            s.to_string()
        }
    }

    pub fn to_download_sections_string(&self) -> String {
        let start = self.start.map(Self::format_time);
        let end = self.end.map(Self::format_time);
        format!("*{}-{}", start.as_deref().unwrap_or("0"), end.as_deref().unwrap_or_default())
    }
}

impl FromStr for Sections {
    type Err = ParseSectionError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        let raw = if let Some(rest) = raw.strip_prefix('*') { rest.trim() } else { raw };
        if raw.is_empty() {
            return Ok(Sections::default());
        }
        let parts: Vec<&str> = raw.split('-').collect();
        if parts.len() > 2 {
            return Err(Self::Err::InvalidFormat);
        }
        let start = Sections::parse_section(parts.get(0).unwrap_or(&""))?;
        let end = Sections::parse_section(parts.get(1).unwrap_or(&""))?;
        let sections = Sections { start, end };
        Ok(sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_range() {
        let s: Sections = "1:00-3:00".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(60),
                end: Some(180),
            }
        );
    }

    #[test]
    fn parse_seconds_range() {
        let s: Sections = "30-90".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(30),
                end: Some(90),
            }
        );
    }

    #[test]
    fn parse_start_only() {
        let s: Sections = "3:00-".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(180),
                end: None,
            }
        );
    }

    #[test]
    fn parse_end_only() {
        let s: Sections = "-3:00".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: None,
                end: Some(180),
            }
        );
    }

    #[test]
    fn parse_full_video_dash_only() {
        let s: Sections = "-".parse().unwrap();
        assert_eq!(s, Sections { start: None, end: None });
    }

    #[test]
    fn parse_with_star_prefix() {
        let s: Sections = "*1:00-2:00".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(60),
                end: Some(120),
            }
        );
    }

    #[test]
    fn parse_with_spaces() {
        let s: Sections = "  *1:00-2:00  ".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(60),
                end: Some(120),
            }
        );
    }

    #[test]
    fn parse_empty_string_defaults() {
        let s: Sections = "".parse().unwrap();
        assert_eq!(s, Sections::default());
    }

    #[test]
    fn parse_invalid_time() {
        assert!("abc-2:00".parse::<Sections>().is_err());
        assert!("1:xx-2:00".parse::<Sections>().is_err());
    }

    #[test]
    fn parse_invalid_format_multiple_dashes() {
        assert!("1-2-3".parse::<Sections>().is_err());
    }

    #[test]
    fn format_full_range() {
        let s = Sections {
            start: Some(60),
            end: Some(150),
        };
        assert_eq!(s.to_download_sections_string(), "*1:00-2:30");
    }

    #[test]
    fn format_start_only() {
        let s = Sections {
            start: Some(90),
            end: None,
        };
        assert_eq!(s.to_download_sections_string(), "*1:30-");
    }

    #[test]
    fn format_end_only() {
        let s = Sections {
            start: None,
            end: Some(120),
        };
        assert_eq!(s.to_download_sections_string(), "*0-2:00");
    }

    #[test]
    fn format_full_video() {
        let s = Sections { start: None, end: None };
        assert_eq!(s.to_download_sections_string(), "*0-");
    }

    #[test]
    fn roundtrip_full_range() {
        let original: Sections = "1:00-2:30".parse().unwrap();
        let formatted = original.to_download_sections_string();
        let reparsed: Sections = formatted.parse().unwrap();

        assert_eq!(original, reparsed);
    }
}
