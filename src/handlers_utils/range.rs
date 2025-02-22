use std::num::ParseIntError;
use std::str::FromStr;
use thiserror::Error;

const MAX_ELEMENTS: i16 = 10;
const DEFAULT_START: i16 = 1;
const DEFAULT_END: i16 = MAX_ELEMENTS;
const DEFAULT_STEP: i16 = 1;

#[derive(Error, Debug, PartialEq)]
pub enum ParseRangeError {
    #[error("Failed to parse number: {0}")]
    InvalidNumber(#[from] ParseIntError),
    #[error("Invalid range format. Expected format \"start\" or \"start:end\" optionally followed by \",step\"")]
    InvalidFormat,
    #[error("Step cannot be zero")]
    ZeroStep,
}

#[derive(Debug, PartialEq)]
pub struct Range {
    pub start: Option<i16>,
    pub end: Option<i16>,
    pub step: Option<i16>,
}

impl Range {
    pub const fn new(start: Option<i16>, end: Option<i16>, step: Option<i16>) -> Self {
        Self { start, end, step }
    }
}

impl Default for Range {
    fn default() -> Self {
        Range {
            start: Some(DEFAULT_START),
            end: Some(DEFAULT_END),
            step: Some(DEFAULT_STEP),
        }
    }
}

impl Range {
    pub fn normalize(&mut self) {
        let start = match self.start {
            Some(s) if s > 0 => s,
            _ => DEFAULT_START,
        };
        let step = match self.step {
            Some(s) if s > 0 => s,
            _ => DEFAULT_STEP,
        };
        let mut end = match self.end {
            Some(e) if e > 0 => e,
            _ => i16::MAX,
        };

        let count = ((end - start) / step).abs() + 1;
        if count > MAX_ELEMENTS {
            end = start + (MAX_ELEMENTS - 1) * step;
        }

        self.start = Some(start);
        self.end = Some(end);
        self.step = Some(step);
    }

    /// Returns a string representation of the range.
    ///
    /// The format follows:
    /// - "start" if only start is provided.
    /// - "start:end" if start and end are provided.
    /// - "start:end,step" if all three values are provided.
    ///
    /// This method first normalizes the range.
    pub fn to_range_string(&mut self) -> String {
        self.normalize();
        let start = self.start.unwrap();
        let end = self.end.unwrap();

        // Build the string representation.
        // If the step is the default value (and hence not explicitly provided),
        // we can use "start:end". Otherwise, include the step.
        if let Some(step) = self.step {
            // Only include the step if it's not the default step or if it's important to show it.
            // For this example, we always include it if it's available.
            format!("{}:{}{}", start, end, format!(",{}", step))
        } else {
            format!("{}:{}", start, end)
        }
    }
}

impl FromStr for Range {
    type Err = ParseRangeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, ',');
        let range_part = parts.next().ok_or(ParseRangeError::InvalidFormat)?;
        let step_part = parts.next();

        let step = if let Some(part) = step_part {
            let parsed = part.trim();
            if parsed.is_empty() {
                None
            } else {
                let step_val: i16 = parsed.parse()?;
                if step_val == 0 {
                    return Err(ParseRangeError::ZeroStep);
                }
                Some(step_val)
            }
        } else {
            None
        };

        let range_vals: Vec<&str> = range_part.split(':').collect();
        if range_vals.len() > 2 {
            return Err(ParseRangeError::InvalidFormat);
        }

        fn parse_val(part: &str) -> Result<Option<i16>, ParseRangeError> {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed.parse::<i16>().map(Some).map_err(ParseRangeError::InvalidNumber)
            }
        }

        let start = if !range_vals.is_empty() { parse_val(range_vals[0])? } else { None };
        let end = if range_vals.len() == 2 { parse_val(range_vals[1])? } else { None };

        Ok(Range { start, end, step })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_only_start() {
        // Test parsing when only "start" is provided.
        // "5" -> start = 5, end = None, step = None.
        // After normalization, start remains 5, end becomes start + (MAX_ELEMENTS - 1)*DEFAULT_STEP = 5 + 9 = 14, step becomes DEFAULT_STEP (1).
        let mut range: Range = "5".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: None,
                step: None
            }
        );
        range.normalize();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(14),
                step: Some(1)
            }
        );
    }

    #[test]
    fn test_parse_start_end() {
        // Test parsing with "start:end" format.
        // "5:50" -> start = 5, end = 50, step = None.
        // Normalization adjusts step to DEFAULT_STEP (1) and, since the count is too high,
        // recalculates end as start + (MAX_ELEMENTS - 1)*1 = 5 + 9 = 14.
        let mut range: Range = "5:50".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(50),
                step: None
            }
        );
        range.normalize();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(14),
                step: Some(1)
            }
        );
    }

    #[test]
    fn test_parse_full_format() {
        // Test parsing full format "start:end,step".
        // "5:50,2" -> start = 5, end = 50, step = 2.
        // Normalization: count = ((50 - 5) / 2) + 1 = 23, which is greater than MAX_ELEMENTS.
        // Hence, end is recalculated as 5 + 9*2 = 23.
        let mut range: Range = "5:50,2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(50),
                step: Some(2)
            }
        );
        range.normalize();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(23),
                step: Some(2)
            }
        );
    }

    #[test]
    fn test_zero_step_error() {
        // Test that explicitly specifying a step of 0 returns an error.
        let err = "5:10,0".parse::<Range>().unwrap_err();
        assert_eq!(err, ParseRangeError::ZeroStep);
    }

    #[test]
    fn test_parse_with_empty_parts() {
        // Test parsing with empty parts.
        // ":" -> both start and end are empty, so they are parsed as None.
        // Normalization substitutes defaults: start becomes DEFAULT_START (1), end becomes MAX but then adjusted due to MAX_ELEMENTS, so end = 1 + 9*DEFAULT_STEP = 10,
        // and step becomes DEFAULT_STEP (1).
        let mut range: Range = ":".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: None,
                end: None,
                step: None
            }
        );
        range.normalize();
        assert_eq!(
            range,
            Range {
                start: Some(1),
                end: Some(10),
                step: Some(1)
            }
        );
    }

    #[test]
    fn test_parse_negative_values() {
        // Test that negative values are replaced with defaults during normalization.
        // "-5:-50,-2" are parsed as negative numbers.
        // Normalization will substitute: start becomes DEFAULT_START (1), end becomes MAX (1000) adjusted by MAX_ELEMENTS to 10, and step becomes DEFAULT_STEP (1).
        let mut range: Range = "-5:-50,-2".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: Some(-5),
                end: Some(-50),
                step: Some(-2)
            }
        );
        range.normalize();
        assert_eq!(
            range,
            Range {
                start: Some(1),
                end: Some(10),
                step: Some(1)
            }
        );
    }

    #[test]
    fn test_invalid_format_extra_colon() {
        // Test an invalid format with too many colons.
        // "5:10:15,2" should return an InvalidFormat error because only one colon is allowed.
        let err = "5:10:15,2".parse::<Range>().unwrap_err();
        assert_eq!(err, ParseRangeError::InvalidFormat);
    }

    #[test]
    fn test_invalid_number() {
        // Test invalid numeric value.
        // "a:10,2" should return an InvalidNumber error.
        let err = "a:10,2".parse::<Range>().unwrap_err();
        if let ParseRangeError::InvalidNumber(_) = err {
            // Expected error, do nothing.
        } else {
            panic!("Expected InvalidNumber error, got {:?}", err);
        }
    }

    #[test]
    fn test_missing_step_value() {
        // Test when a comma is present but the step value is missing.
        // "5:10," -> step should be considered as None and then normalized to DEFAULT_STEP (1).
        let mut range: Range = "5:10,".parse().unwrap();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(10),
                step: None
            }
        );
        range.normalize();
        assert_eq!(
            range,
            Range {
                start: Some(5),
                end: Some(10),
                step: Some(1)
            }
        );
    }
}
