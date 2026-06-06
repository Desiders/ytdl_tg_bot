use serde::{Deserialize, Serialize};

use crate::database::models::sea_orm_active_enums::MediaType as Model;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum MediaType {
    Video,
    Audio,
    Photo,
}

impl From<Model> for MediaType {
    fn from(value: Model) -> Self {
        match value {
            Model::Video => MediaType::Video,
            Model::Audio => MediaType::Audio,
            Model::Photo => MediaType::Photo,
        }
    }
}

impl From<MediaType> for Model {
    fn from(value: MediaType) -> Self {
        match value {
            MediaType::Video => Model::Video,
            MediaType::Audio => Model::Audio,
            MediaType::Photo => Model::Photo,
        }
    }
}
