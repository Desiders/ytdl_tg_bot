use std::num::ParseIntError;
use std::str::FromStr;
use thiserror::Error;

const MAX_ELEMENTS: i16 = 10;
const DEFAULT_START: i16 = 1;
const DEFAULT_COUNT: i16 = 1;
const DEFAULT_STEP: i16 = 1;

#[derive(Error, Debug, PartialEq)]
pub enum ParseRangeError {
    #[error("Failed to parse number: {0}")]
    InvalidNumber(#[from] ParseIntError),
    #[error("Invalid range format. Expected format \"start:count:step\" or variations")]
    InvalidFormat,
    #[error("Step cannot be zero")]
    ZeroStep,
}

#[derive(Debug, PartialEq)]
pub struct Range {
    pub start: i16,
    pub count: i16,
    pub step: i16,
}

impl Range {
    pub fn normalize(&mut self) {
        let count = ((self.count - self.start) / self.step).abs() + 1;
        if count > MAX_ELEMENTS {
            self.count = self.start + (MAX_ELEMENTS * self.step) - self.step;
        }
    }

    pub const fn get_element_count(&self) -> i16 {
        ((self.count - self.start) / self.step).abs() + 1
    }

    pub const fn is_single_element(&self) -> bool {
        self.get_element_count() == 1
    }

    pub fn to_range_string(&self) -> String {
        format!("{}:{}:{}", self.start, self.count, self.step)
    }
}

impl Default for Range {
    fn default() -> Self {
        Range {
            start: DEFAULT_START,
            count: DEFAULT_COUNT,
            step: DEFAULT_STEP,
        }
    }
}

fn parse_optional_positive(part: &str, is_step: bool) -> Result<Option<i16>, ParseRangeError> {
    if part.trim().is_empty() {
        return Ok(None);
    }
    let n = part.trim().parse::<i16>().map_err(ParseRangeError::InvalidNumber)?;
    if n == 0 && is_step {
        return Err(ParseRangeError::ZeroStep);
    }
    if n <= 0 {
        return Ok(None);
    }
    Ok(Some(n))
}

impl FromStr for Range {
    type Err = ParseRangeError;

    #[allow(clippy::get_first, clippy::similar_names)]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Ok(Range::default());
        }
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() > 3 {
            return Err(ParseRangeError::InvalidFormat);
        }
        let start = parse_optional_positive(parts.get(0).unwrap_or(&""), false)?.unwrap_or(DEFAULT_START);
        let step = parse_optional_positive(parts.get(2).unwrap_or(&""), true)?.unwrap_or(DEFAULT_STEP);
        let count = parse_optional_positive(parts.get(1).unwrap_or(&""), false)?.unwrap_or(start + (MAX_ELEMENTS * step) - step);
        let mut range = Range { start, count, step };
        range.normalize();
        Ok(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_range() {
        let range = Range::default();
        assert_eq!(
            range,
            Range {
                start: 1,
                count: 1,
                step: 1
            }
        );
    }

    #[test]
    fn test_parse_only_start() {
        let range: Range = "5".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 5,
                count: 14,
                step: 1
            }
        );
    }

    #[test]
    fn test_parse_start_stop() {
        let range: Range = "5:50".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 5,
                count: 14,
                step: 1
            }
        );
    }

    #[test]
    fn test_parse_full_format() {
        let range: Range = "5:50:2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 5,
                count: 23,
                step: 2
            }
        );
    }

    #[test]
    fn test_parse_negative_values() {
        let range: Range = "-5:-50:-2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 1,
                count: 10,
                step: 1
            }
        );
    }

    #[test]
    fn test_parse_with_empty_parts() {
        let range: Range = ":".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 1,
                count: 10,
                step: 1
            }
        );
    }
    #[test]
    fn test_parse_missing_step_value() {
        let range: Range = "5:10:".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 5,
                count: 10,
                step: 1
            }
        );
    }

    #[test]
    fn test_parse_no_start_no_end() {
        let range: Range = "::2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 1,
                count: 19,
                step: 2
            }
        );
    }

    #[test]
    fn test_parse_no_end() {
        let range: Range = "5::2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 5,
                count: 23,
                step: 2
            }
        );
    }

    #[test]
    fn test_realistic_range_cases() {
        let range: Range = "10:30:5".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 10,
                count: 30,
                step: 5
            }
        );
        let range: Range = "3:15:3".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 3,
                count: 15,
                step: 3
            }
        );
    }

    #[test]
    fn test_to_range_string() {
        let range = Range {
            start: 3,
            count: 15,
            step: 2,
        };
        assert_eq!(range.to_range_string(), "3:15:2");
    }

    #[test]
    fn test_parse_no_end_with_non_default_step() {
        let range: Range = "2::2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 2,
                count: 20,
                step: 2
            }
        );
    }

    #[test]
    fn test_parse_no_start_and_end_with_non_default_step() {
        let range: Range = "::2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 1,
                count: 19,
                step: 2
            }
        );
    }

    #[test]
    fn test_parse_no_start_with_non_default_step() {
        let range: Range = ":10:2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: 1,
                count: 10,
                step: 2
            }
        );
    }

    #[test]
    fn test_get_element_count() {
        let range_a = Range {
            start: 1,
            count: 10,
            step: 1,
        };
        let range_b = Range {
            start: 1,
            count: 10,
            step: 2,
        };

        let range_c = Range {
            start: 1,
            count: 5,
            step: 2,
        };

        assert_eq!(range_a.get_element_count(), 10);
        assert_eq!(range_b.get_element_count(), 5);
        assert_eq!(range_c.get_element_count(), 3);
    }
}
