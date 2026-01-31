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
                    .drop_foreign_key("idx_downloaded_media_id_domain_type")
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .add_column(string_null(DownloadedMedia::AudioLanguage).default(Expr::null()))
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE downloaded_media ADD CONSTRAINT idx_downloaded_media_id_domain_type_lang UNIQUE (id, domain, media_type, audio_language)"
            ).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .drop_foreign_key(Alias::new("idx_downloaded_media_id_domain_type_lang"))
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .drop_column(DownloadedMedia::AudioLanguage)
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE downloaded_media ADD CONSTRAINT idx_downloaded_media_id_domain_type UNIQUE (id, domain, media_type)",
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum DownloadedMedia {
    Table,
    AudioLanguage,
}
