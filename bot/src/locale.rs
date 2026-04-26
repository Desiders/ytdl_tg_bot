use std::fmt;

pub const DEFAULT: &str = "en";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    En,
    Ru,
}

impl Locale {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ru => "ru",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "en" | "english" | "английский" => Some(Self::En),
            "ru" | "russian" | "ру" | "русский" => Some(Self::Ru),
            _ => None,
        }
    }

    pub fn from_telegram_code(code: Option<&str>) -> Self {
        match code {
            Some(c) if c.eq_ignore_ascii_case("ru") || c.to_ascii_lowercase().starts_with("ru") => Self::Ru,
            _ => Self::En,
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::En => Self::Ru,
            Self::Ru => Self::En,
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for Locale {
    fn from(s: &str) -> Self {
        Self::parse(s).unwrap_or_default()
    }
}
