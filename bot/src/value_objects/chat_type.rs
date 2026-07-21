use crate::database::models::sea_orm_active_enums::ChatType as Model;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatType {
    Private,
    Group,
    Supergroup,
    Channel,
}

impl From<Model> for ChatType {
    fn from(value: Model) -> Self {
        match value {
            Model::Private => Self::Private,
            Model::Group => Self::Group,
            Model::Supergroup => Self::Supergroup,
            Model::Channel => Self::Channel,
        }
    }
}

impl From<ChatType> for Model {
    fn from(value: ChatType) -> Self {
        match value {
            ChatType::Private => Self::Private,
            ChatType::Group => Self::Group,
            ChatType::Supergroup => Self::Supergroup,
            ChatType::Channel => Self::Channel,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Unsupported chat type: {0}")]
pub struct UnsupportedChatTypeError(pub &'static str);

impl TryFrom<telers::enums::ChatType> for ChatType {
    type Error = UnsupportedChatTypeError;

    fn try_from(value: telers::enums::ChatType) -> Result<Self, Self::Error> {
        match value {
            telers::enums::ChatType::Private => Ok(Self::Private),
            telers::enums::ChatType::Group => Ok(Self::Group),
            telers::enums::ChatType::Supergroup => Ok(Self::Supergroup),
            telers::enums::ChatType::Channel => Ok(Self::Channel),
            value @ telers::enums::ChatType::Unknown => Err(UnsupportedChatTypeError(value.into())),
        }
    }
}
