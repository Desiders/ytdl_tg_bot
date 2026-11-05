use sea_orm_migration::{async_trait::async_trait, prelude::*, schema::text_null};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .add_column(text_null(DownloadedMedia::Description))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .drop_column(DownloadedMedia::Description)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum DownloadedMedia {
    Table,
    Description,
}
