use sea_orm_migration::{
    async_trait::async_trait,
    prelude::*,
    schema::{
        big_integer, big_integer_null, double_null, float_null, integer, string, string_null, text, text_null, timestamp_with_time_zone,
        uuid,
    },
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MediaCatalogs::Table)
                    .col(uuid(MediaCatalogs::Id).primary_key())
                    .col(text(MediaCatalogs::Url))
                    .col(string(MediaCatalogs::AudioLanguage))
                    .col(big_integer(MediaCatalogs::MaxFileSize))
                    .col(text(MediaCatalogs::MediaId))
                    .col(text(MediaCatalogs::WebpageUrl))
                    .col(text(MediaCatalogs::Title))
                    .col(text_null(MediaCatalogs::Description))
                    .col(float_null(MediaCatalogs::Duration))
                    .col(timestamp_with_time_zone(MediaCatalogs::UpdatedAt))
                    .index(
                        Index::create()
                            .name("idx_media_catalogs_url_lang_size")
                            .col(MediaCatalogs::Url)
                            .col(MediaCatalogs::AudioLanguage)
                            .col(MediaCatalogs::MaxFileSize)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(MediaFormats::Table)
                    .col(big_integer(MediaFormats::Id).auto_increment().primary_key())
                    .col(uuid(MediaFormats::CatalogId))
                    .col(text(MediaFormats::FormatId))
                    .col(integer(MediaFormats::Position))
                    .col(string(MediaFormats::Ext))
                    .col(big_integer(MediaFormats::Width))
                    .col(big_integer(MediaFormats::Height))
                    .col(double_null(MediaFormats::Fps))
                    .col(string_null(MediaFormats::Vcodec))
                    .col(string_null(MediaFormats::Acodec))
                    .col(big_integer_null(MediaFormats::FilesizeApprox))
                    .index(
                        Index::create()
                            .name("idx_media_formats_catalog_format")
                            .col(MediaFormats::CatalogId)
                            .col(MediaFormats::FormatId)
                            .unique(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_formats_catalog")
                            .from(MediaFormats::Table, MediaFormats::CatalogId)
                            .to(MediaCatalogs::Table, MediaCatalogs::Id)
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(MediaFormats::Table).to_owned()).await?;
        manager.drop_table(Table::drop().table(MediaCatalogs::Table).to_owned()).await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum MediaCatalogs {
    Table,
    Id,
    Url,
    AudioLanguage,
    MaxFileSize,
    MediaId,
    WebpageUrl,
    Title,
    Description,
    Duration,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum MediaFormats {
    Table,
    Id,
    CatalogId,
    FormatId,
    Position,
    Ext,
    Width,
    Height,
    Fps,
    Vcodec,
    Acodec,
    FilesizeApprox,
}
