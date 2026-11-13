use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "media_formats")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique_key = "idx_media_formats_catalog_format")]
    pub catalog_id: Uuid,
    #[sea_orm(unique_key = "idx_media_formats_catalog_format")]
    pub format_id: String,
    pub position: i32,
    pub ext: String,
    pub width: i64,
    pub height: i64,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub filesize_approx: Option<i64>,
    #[sea_orm(belongs_to, from = "catalog_id", to = "id", on_update = "Cascade", on_delete = "Cascade")]
    pub media_catalogs: HasOne<super::media_catalogs::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
