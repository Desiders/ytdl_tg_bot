use sea_orm::{DeriveActiveEnum, EnumIter};

use crate::value_objects::MediaType as MediaTypeVO;

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter, DeriveActiveEnum)]
#[sea_orm(rs_type = "String", db_type = "Enum", enum_name = "media_type")]
pub enum MediaType {
    #[sea_orm(string_value = "video")]
    Video,
    #[sea_orm(string_value = "audio")]
    Audio,
}

impl From<MediaType> for MediaTypeVO {
    fn from(value: MediaType) -> Self {
        match value {
            MediaType::Video => Self::Video,
            MediaType::Audio => Self::Audio,
        }
    }
}

impl From<MediaTypeVO> for MediaType {
    fn from(value: MediaTypeVO) -> Self {
        match value {
            MediaTypeVO::Video => Self::Video,
            MediaTypeVO::Audio => Self::Audio,
        }
    }
}
