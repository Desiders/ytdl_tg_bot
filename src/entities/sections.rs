use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum ParseSectionError {
    #[error("Invalid time format")]
    InvalidTime,
    #[error("Invalid section format. Expected \"start-end\".")]
    InvalidFormat,
}

#[derive(Default, Debug, PartialEq)]
pub struct Sections {
    pub start: Option<u32>,
    pub end: Option<u32>,
}

impl Sections {
    fn parse_section(raw: &str) -> Result<Option<u32>, ParseSectionError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }

        let parts: Vec<&str> = raw.split(':').collect();
        let secs = match parts.len() {
            1 => parts[0].parse::<u32>().map_err(|_| ParseSectionError::InvalidTime)?,
            2 => {
                let m = parts[0].parse::<u32>().map_err(|_| ParseSectionError::InvalidTime)?;
                let s = parts[1].parse::<u32>().map_err(|_| ParseSectionError::InvalidTime)?;
                m * 60 + s
            }
            3 => {
                let h = parts[0].parse::<u32>().map_err(|_| ParseSectionError::InvalidTime)?;
                let m = parts[1].parse::<u32>().map_err(|_| ParseSectionError::InvalidTime)?;
                let s = parts[2].parse::<u32>().map_err(|_| ParseSectionError::InvalidTime)?;
                h * 3600 + m * 60 + s
            }
            _ => return Err(ParseSectionError::InvalidTime),
        };

        Ok(Some(secs))
    }

    fn format_time(t: u32) -> String {
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
        if self.start.is_none() && self.end.is_none() {
            return "-".to_string();
        }

        let start = self.start.map(Self::format_time).unwrap_or_default();
        let end = self.end.map(Self::format_time).unwrap_or_default();
        format!("*{start}-{end}")
    }
}

impl FromStr for Sections {
    type Err = ParseSectionError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.trim();
        let raw = if let Some(rest) = raw.strip_prefix('*') { rest.trim() } else { raw };
        if raw.is_empty() {
            return Err(ParseSectionError::InvalidFormat);
        }

        let mut parts = raw.splitn(2, '-');
        let start_raw = parts.next().ok_or(ParseSectionError::InvalidFormat)?;
        let end_raw = parts.next().ok_or(ParseSectionError::InvalidFormat)?;

        let start = Sections::parse_section(start_raw)?;
        let end = Sections::parse_section(end_raw)?;

        Ok(Sections { start, end })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_section() {
        let s: Sections = "*1:00-2:30".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(60),
                end: Some(150)
            }
        );
    }

    #[test]
    fn parse_seconds() {
        let s: Sections = "*30-90".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(30),
                end: Some(90)
            }
        );
    }

    #[test]
    fn parse_start_only() {
        let s: Sections = "*1:20-".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(80),
                end: None
            }
        );
    }

    #[test]
    fn parse_end_only() {
        let s: Sections = "*-2:00".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: None,
                end: Some(120)
            }
        );
    }

    #[test]
    fn parse_without_star() {
        let s: Sections = "10-20".parse().unwrap();
        assert_eq!(
            s,
            Sections {
                start: Some(10),
                end: Some(20)
            }
        );
    }

    #[test]
    fn invalid_format() {
        assert!("abc".parse::<Sections>().is_err());
    }

    #[test]
    fn test_to_download_sections_string() {
        let s = Sections { start: None, end: None };
        assert_eq!(s.to_download_sections_string(), "-");

        let s = Sections {
            start: Some(60),
            end: Some(150),
        };
        assert_eq!(s.to_download_sections_string(), "*1:00-2:30");

        let s = Sections {
            start: Some(30),
            end: Some(90),
        };
        assert_eq!(s.to_download_sections_string(), "*30-1:30");

        let s = Sections {
            start: Some(80),
            end: None,
        };
        assert_eq!(s.to_download_sections_string(), "*1:20-");

        let s = Sections {
            start: None,
            end: Some(120),
        };
        assert_eq!(s.to_download_sections_string(), "*-2:00");
    }
}
