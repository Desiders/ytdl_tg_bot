use sea_orm::{ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EnumIter, PrimaryKeyTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{database::enums::MediaType, entities::DownloadedMedia};

#[derive(Debug, Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "downloaded_media")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub tg_id: i64,
    pub url: String,
    pub id_in_url: Option<String>,
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
            tg_id,
            url,
            id_in_url,
            media_type,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            tg_id,
            url: url.into_boxed_str(),
            id_in_url: id_in_url.map(String::into_boxed_str),
            media_type: media_type.into(),
            created_at,
        }
    }
}
