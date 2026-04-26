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
            "en" => Some(Self::En),
            "ru" => Some(Self::Ru),
            _ => None,
        }
    }

    pub fn from_telegram_code(code: Option<&str>) -> Self {
        match code {
            Some(c) if c.to_ascii_lowercase().starts_with("ru") => Self::Ru,
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

impl From<&str> for Locale {
    fn from(s: &str) -> Self {
        Self::parse(s).unwrap_or_default()
    }
}
