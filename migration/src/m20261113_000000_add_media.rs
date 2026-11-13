use sea_orm_migration::{
    async_trait::async_trait,
    prelude::*,
    schema::{big_integer, big_integer_null, double_null, integer, string, string_null, text, text_null, timestamp_with_time_zone, uuid},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Media::Table)
                    .col(uuid(Media::Id).primary_key())
                    .col(text(Media::Url))
                    .col(string(Media::AudioLanguage))
                    .col(big_integer(Media::MaxFileSize))
                    .col(text(Media::Title))
                    .col(text_null(Media::Description))
                    .col(timestamp_with_time_zone(Media::UpdatedAt))
                    .index(
                        Index::create()
                            .name("idx_media_url_lang_size")
                            .col(Media::Url)
                            .col(Media::AudioLanguage)
                            .col(Media::MaxFileSize)
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
                    .col(uuid(MediaFormats::MediaId))
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
                            .name("idx_media_formats_media_format")
                            .col(MediaFormats::MediaId)
                            .col(MediaFormats::FormatId)
                            .unique(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_media_formats_media")
                            .from(MediaFormats::Table, MediaFormats::MediaId)
                            .to(Media::Table, Media::Id)
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
        manager.drop_table(Table::drop().table(Media::Table).to_owned()).await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Media {
    Table,
    Id,
    Url,
    AudioLanguage,
    MaxFileSize,
    Title,
    Description,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum MediaFormats {
    Table,
    Id,
    MediaId,
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
