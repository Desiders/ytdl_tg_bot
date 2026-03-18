use std::{convert::Infallible, str::FromStr};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Language {
    pub language: Option<String>,
}

impl FromStr for Language {
    type Err = Infallible;

    fn from_str(val: &str) -> Result<Self, Self::Err> {
        if val.trim().is_empty() {
            return Ok(Self::default());
        }
        Ok(Self {
            language: Some(val.to_owned()),
        })
    }
}
