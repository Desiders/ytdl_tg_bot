use crate::database::models::sea_orm_active_enums::MediaType as Model;

#[derive(Debug)]
pub enum MediaType {
    Video,
    Audio,
}

impl From<Model> for MediaType {
    fn from(value: Model) -> Self {
        match value {
            Model::Video => MediaType::Video,
            Model::Audio => MediaType::Audio,
        }
    }
}

impl From<MediaType> for Model {
    fn from(value: MediaType) -> Self {
        match value {
            MediaType::Video => Model::Video,
            MediaType::Audio => Model::Audio,
        }
    }
}
