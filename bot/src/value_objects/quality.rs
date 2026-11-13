use telers::{CallbackData, CallbackDataError, CallbackDataValue};

#[derive(Debug, Clone, Copy)]
pub enum QualityAction {
    Best,
    Resolution(i64),
    Video(i64),
    FormatsPage { resolution: i64, page: usize },
    Page(usize),
    Cancel,
}

impl CallbackDataValue for QualityAction {
    fn encode(&self) -> Result<String, CallbackDataError> {
        Ok(match self {
            Self::Best => "best".to_owned(),
            Self::Resolution(index) => format!("r{index}"),
            Self::FormatsPage { resolution, page } => format!("f{resolution}.{page}"),
            Self::Video(index) => format!("v{index}"),
            Self::Page(index) => format!("p{index}"),
            Self::Cancel => "cancel".to_owned(),
        })
    }

    fn decode(value: &str, field: &'static str) -> Result<Self, CallbackDataError> {
        match value {
            "best" => return Ok(Self::Best),
            "cancel" => return Ok(Self::Cancel),
            _ => {}
        }
        let invalid = || CallbackDataError::InvalidValue {
            field,
            value: value.into(),
        };
        if let Some(page) = value.strip_prefix('f') {
            let (resolution, page) = page.split_once('.').ok_or_else(invalid)?;
            return Ok(Self::FormatsPage {
                resolution: resolution.parse().map_err(|_| invalid())?,
                page: page.parse().map_err(|_| invalid())?,
            });
        }
        let index: usize = value.get(1..).ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
        match value.get(..1) {
            Some("r") => Ok(Self::Resolution(i64::try_from(index).map_err(|_| invalid())?)),
            Some("v") => Ok(Self::Video(i64::try_from(index).map_err(|_| invalid())?)),
            Some("p") => Ok(Self::Page(index)),
            _ => Err(invalid()),
        }
    }
}

#[derive(Debug, Clone, CallbackData)]
#[callback_data(prefix = "quality")]
pub struct SelectQuality {
    pub session: String,
    pub action: QualityAction,
}
