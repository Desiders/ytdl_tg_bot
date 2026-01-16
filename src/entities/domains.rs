use std::{convert::Infallible, str::FromStr};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Domains {
    pub domains: Box<[String]>,
}

impl AsRef<[String]> for Domains {
    fn as_ref(&self) -> &[String] {
        &self.domains
    }
}

impl FromStr for Domains {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Ok(Domains::default());
        }

        let domains = s.split('|').filter(|part| !part.is_empty()).map(|part| part.to_owned()).collect();
        Ok(Self { domains })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_format() {
        let domains: Domains = "youtube.com|youtu.be".parse().unwrap();
        assert_eq!(
            domains,
            Domains {
                domains: Box::new(["youtube.com".into(), "youtu.be".into()]),
            }
        );
    }

    #[test]
    fn test_parse_with_empty_parts() {
        let domains: Domains = "|".parse().unwrap();
        assert_eq!(domains, Domains { domains: Box::new([]) });
    }

    #[test]
    fn test_parse_missing_step_value() {
        let domains: Domains = "youtube.com|".parse().unwrap();
        assert_eq!(
            domains,
            Domains {
                domains: Box::new(["youtube.com".into()]),
            }
        );
    }
}
