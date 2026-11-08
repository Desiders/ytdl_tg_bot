use telers::{CallbackData, CallbackDataError, CallbackDataValue};

#[derive(Debug, Clone, Copy)]
pub enum QualityAction {
    Best,
    Video(usize),
    Page(usize),
    Cancel,
}

impl CallbackDataValue for QualityAction {
    fn encode(&self) -> Result<String, CallbackDataError> {
        Ok(match self {
            Self::Best => "best".to_owned(),
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
        let index = value.get(1..).ok_or_else(invalid)?.parse().map_err(|_| invalid())?;
        match value.get(..1) {
            Some("v") => Ok(Self::Video(index)),
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
