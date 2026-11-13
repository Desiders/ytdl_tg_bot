use sea_orm::entity::prelude::*;

#[sea_orm::model]
#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "media")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    #[sea_orm(unique_key = "idx_media_url_lang_size")]
    pub url: String,
    #[sea_orm(unique_key = "idx_media_url_lang_size")]
    pub audio_language: String,
    #[sea_orm(unique_key = "idx_media_url_lang_size")]
    pub max_file_size: i64,
    pub title: String,
    pub description: Option<String>,
    pub updated_at: TimeDateTimeWithTimeZone,
    #[sea_orm(has_many)]
    pub media_formats: HasMany<super::media_formats::Entity>,
}

impl ActiveModelBehavior for ActiveModel {}
