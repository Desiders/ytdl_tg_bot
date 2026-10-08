use telers::{CallbackData, CallbackDataError, CallbackDataValue};

use crate::locale::Locale;

/// A private-chat menu screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Settings,
    Language,
    Help,
    HelpCommands,
    HelpArguments,
    HelpInline,
    Stats,
    Domains,
    DomainAdd,
    Domain { key: DomainKey },
    DomainDelete { key: DomainKey },
}

/// A menu step that waits for the next message instead of a button press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuState {
    DomainInput,
}

impl AsRef<str> for MenuState {
    fn as_ref(&self) -> &str {
        match self {
            Self::DomainInput => "menu_domain_input",
        }
    }
}

impl PartialEq<&str> for MenuState {
    fn eq(&self, other: &&str) -> bool {
        self.as_ref() == *other
    }
}

// Callback data is capped at 64 bytes, too short for a domain, so buttons name it by a fingerprint. A list position
//  would point at another domain once the list changes under an old message.
//  https://core.telegram.org/bots/api#inlinekeyboardbutton
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainKey(u64);

impl DomainKey {
    // FNV-1a rather than `DefaultHasher`, whose hashes may change between Rust releases and break old buttons.
    //  https://doc.rust-lang.org/std/collections/hash_map/struct.DefaultHasher.html#method.new
    #[must_use]
    pub fn new(domain: &str) -> Self {
        let hash = domain.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
        Self(hash)
    }

    #[must_use]
    pub fn position(self, domains: &[String]) -> Option<usize> {
        domains.iter().position(|domain| Self::new(domain) == self)
    }
}

impl CallbackDataValue for DomainKey {
    fn encode(&self) -> Result<String, CallbackDataError> {
        Ok(format!("{:016x}", self.0))
    }

    fn decode(value: &str, field: &'static str) -> Result<Self, CallbackDataError> {
        u64::from_str_radix(value, 16)
            .map(Self)
            .map_err(|_| CallbackDataError::InvalidValue {
                field,
                value: value.into(),
            })
    }
}

impl CallbackDataValue for Screen {
    fn encode(&self) -> Result<String, CallbackDataError> {
        Ok(match self {
            Self::Main => "main".to_owned(),
            Self::Settings => "settings".to_owned(),
            Self::Language => "language".to_owned(),
            Self::Help => "help".to_owned(),
            Self::HelpCommands => "help_commands".to_owned(),
            Self::HelpArguments => "help_arguments".to_owned(),
            Self::HelpInline => "help_inline".to_owned(),
            Self::Stats => "stats".to_owned(),
            Self::Domains => "domains".to_owned(),
            Self::DomainAdd => "domain_add".to_owned(),
            Self::Domain { key } => format!("domain.{}", key.encode()?),
            Self::DomainDelete { key } => format!("domain_delete.{}", key.encode()?),
        })
    }

    fn decode(value: &str, field: &'static str) -> Result<Self, CallbackDataError> {
        let invalid = || CallbackDataError::InvalidValue {
            field,
            value: value.into(),
        };
        let screen = match value {
            "main" => Self::Main,
            "settings" => Self::Settings,
            "language" => Self::Language,
            "help" => Self::Help,
            "help_commands" => Self::HelpCommands,
            "help_arguments" => Self::HelpArguments,
            "help_inline" => Self::HelpInline,
            "stats" => Self::Stats,
            "domains" => Self::Domains,
            "domain_add" => Self::DomainAdd,
            _ => {
                let (name, key) = value.split_once('.').ok_or_else(invalid)?;
                let key = DomainKey::decode(key, field)?;
                match name {
                    "domain" => Self::Domain { key },
                    "domain_delete" => Self::DomainDelete { key },
                    _ => return Err(invalid()),
                }
            }
        };
        Ok(screen)
    }
}

impl CallbackDataValue for Locale {
    fn encode(&self) -> Result<String, CallbackDataError> {
        Ok(self.as_str().to_owned())
    }

    fn decode(value: &str, field: &'static str) -> Result<Self, CallbackDataError> {
        Self::parse(value).ok_or_else(|| CallbackDataError::InvalidValue {
            field,
            value: value.into(),
        })
    }
}

#[derive(Debug, Clone, CallbackData)]
#[callback_data(prefix = "open")]
pub struct OpenScreen {
    pub screen: Screen,
}

#[derive(Debug, Clone, CallbackData)]
#[callback_data(prefix = "set_language")]
pub struct SetLanguage {
    pub locale: Locale,
}

#[derive(Debug, Clone, CallbackData)]
#[callback_data(prefix = "set_link_visibility")]
pub struct SetLinkVisibility {
    pub visible: bool,
}

#[derive(Debug, Clone, CallbackData)]
#[callback_data(prefix = "delete_domain")]
pub struct DeleteDomain {
    pub domain: DomainKey,
}

#[cfg(test)]
mod tests {
    use super::*;
    use telers::callback_data::CallbackData as _;

    #[test]
    fn every_screen_survives_a_round_trip() {
        for screen in [
            Screen::Main,
            Screen::Settings,
            Screen::Language,
            Screen::Help,
            Screen::HelpCommands,
            Screen::HelpArguments,
            Screen::HelpInline,
            Screen::Stats,
            Screen::Domains,
            Screen::DomainAdd,
            Screen::Domain {
                key: DomainKey::new("first.example"),
            },
            Screen::DomainDelete { key: DomainKey(u64::MAX) },
        ] {
            let packed = OpenScreen { screen }.pack().unwrap();

            assert_eq!(OpenScreen::unpack(&packed).unwrap().screen, screen, "{packed}");
        }
    }

    #[test]
    fn unknown_screens_are_rejected() {
        for packed in [
            "open:unknown",
            "open:domain.x",
            "open:domain_add.1",
            "open:domain",
            "open:domain.-1",
        ] {
            assert!(OpenScreen::unpack(packed).is_err(), "{packed}");
        }
    }

    #[test]
    fn locale_survives_a_round_trip() {
        let packed = SetLanguage { locale: Locale::Uk }.pack().unwrap();

        assert_eq!(packed, "set_language:uk");
        assert_eq!(SetLanguage::unpack(&packed).unwrap().locale, Locale::Uk);
    }

    #[test]
    fn domain_key_is_stable_across_builds() {
        assert_eq!(DomainKey::new("first.example").encode().unwrap(), "06a03322c8e938f9");
    }

    #[test]
    fn an_old_delete_button_matches_only_the_domain_it_showed() {
        let packed = DeleteDomain {
            domain: DomainKey::new("first.example"),
        }
        .pack()
        .unwrap();
        let domain = DeleteDomain::unpack(&packed).unwrap().domain;

        assert_eq!(domain.position(&["second.example".to_owned()]), None);
        assert_eq!(domain.position(&["new.example".to_owned(), "first.example".to_owned()]), Some(1));
    }
}
