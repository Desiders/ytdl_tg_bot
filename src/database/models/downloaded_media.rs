use sea_orm::{ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EnumIter, PrimaryKeyTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{database::enums::MediaType, entities::DownloadedMedia};

#[derive(Debug, Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "downloaded_media")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub file_id: String,
    pub url_or_id: String,
    pub media_type: MediaType,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for DownloadedMedia {
    fn from(
        Model {
            id,
            file_id,
            url_or_id,
            media_type,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            file_id: file_id.into_boxed_str(),
            url_or_id: url_or_id.into_boxed_str(),
            media_type: media_type.into(),
            created_at,
        }
    }
}
