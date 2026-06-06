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

impl From<telers::enums::ChatType> for ChatType {
    fn from(value: telers::enums::ChatType) -> Self {
        match value {
            telers::enums::ChatType::Private => Self::Private,
            telers::enums::ChatType::Group => Self::Group,
            telers::enums::ChatType::Supergroup => Self::Supergroup,
            telers::enums::ChatType::Channel => Self::Channel,
        }
    }
}
