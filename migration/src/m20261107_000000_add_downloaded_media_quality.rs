use sea_orm_migration::{async_trait::async_trait, prelude::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "ALTER TABLE downloaded_media
             ADD COLUMN video_quality text NOT NULL DEFAULT '',
             DROP CONSTRAINT downloaded_media_pkey,
             ADD PRIMARY KEY (file_id, video_quality),
             DROP CONSTRAINT idx_downloaded_media_id_domain_type_lang_sections,
             ADD CONSTRAINT idx_downloaded_media_id_domain_type_lang_sections_quality
             UNIQUE (id, domain, media_type, audio_language, crop_start_time, crop_end_time, video_quality)",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Explicit variants cannot share the old cache key. Preserve the automatic cache on rollback.
        manager
            .get_connection()
            .execute_unprepared(
                "DELETE FROM downloaded_media WHERE video_quality <> '';
             ALTER TABLE downloaded_media
             DROP CONSTRAINT idx_downloaded_media_id_domain_type_lang_sections_quality,
             DROP CONSTRAINT downloaded_media_pkey,
             DROP COLUMN video_quality,
             ADD PRIMARY KEY (file_id),
             ADD CONSTRAINT idx_downloaded_media_id_domain_type_lang_sections
             UNIQUE (id, domain, media_type, audio_language, crop_start_time, crop_end_time)",
            )
            .await?;
        Ok(())
    }
}
