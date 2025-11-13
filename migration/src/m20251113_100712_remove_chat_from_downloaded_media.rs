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
                    .drop_column(DownloadedMedia::ChatTgId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(DownloadedMedia::Table)
                    .add_column(big_integer(DownloadedMedia::ChatTgId).null().default(Expr::null()))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum DownloadedMedia {
    Table,
    ChatTgId,
}
