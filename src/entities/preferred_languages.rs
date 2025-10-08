use std::{convert::Infallible, str::FromStr};

#[derive(Debug, Clone, PartialEq)]
pub struct PreferredLanguages {
    pub languages: Box<[Box<str>]>,
}

impl AsRef<[Box<str>]> for PreferredLanguages {
    fn as_ref(&self) -> &[Box<str>] {
        &self.languages
    }
}

impl Default for PreferredLanguages {
    fn default() -> Self {
        PreferredLanguages {
            languages: Box::new(["ru".into(), "en".into(), "en-US".into(), "en-GB".into()]),
        }
    }
}

impl FromStr for PreferredLanguages {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Ok(PreferredLanguages::default());
        }
        let parts: Vec<&str> = s.split('|').collect();

        let mut languages = Vec::with_capacity(parts.len());
        for part in parts {
            languages.push(part);
        }

        Ok(Self {
            languages: languages.into_iter().map(|val| val.to_owned().into_boxed_str()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_format() {
        let languages: PreferredLanguages = "ru,en".parse().unwrap();
        assert_eq!(
            languages,
            PreferredLanguages {
                languages: Box::new(["ru".into(), "en".into()]),
            }
        );
    }

    #[test]
    fn test_parse_with_empty_parts() {
        let languages: PreferredLanguages = ":".parse().unwrap();
        assert_eq!(languages, PreferredLanguages { languages: Box::new([]) });
    }
    #[test]
    fn test_parse_missing_step_value() {
        let languages: PreferredLanguages = "ru,".parse().unwrap();
        assert_eq!(
            languages,
            PreferredLanguages {
                languages: Box::new(["ru".into()]),
            }
        );
    }
}
