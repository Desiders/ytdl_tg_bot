use sea_orm_migration::{async_trait::async_trait, prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .drop_foreign_key("idx_downloaded_media_id_domain_type_lang")
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .add_column(integer_null(DownloadedMedia::CropStartTime).default(Expr::null()))
                    .add_column(integer_null(DownloadedMedia::CropEndTime).default(Expr::null()))
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE downloaded_media ADD CONSTRAINT \
                idx_downloaded_media_id_domain_type_lang_sections UNIQUE (id, domain, media_type, audio_language, crop_start_time, crop_end_time)"
            ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .drop_foreign_key(Alias::new("idx_downloaded_media_id_domain_type_lang_sections"))
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .drop_column(DownloadedMedia::CropStartTime)
                    .drop_column(DownloadedMedia::CropEndTime)
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE downloaded_media ADD CONSTRAINT idx_downloaded_media_id_domain_type_lang UNIQUE (id, domain, media_type, audio_language)",
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum DownloadedMedia {
    Table,
    CropStartTime,
    CropEndTime,
}
