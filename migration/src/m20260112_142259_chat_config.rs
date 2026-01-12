use sea_orm_migration::{async_trait::async_trait, prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChatConfig::Table)
                    .if_not_exists()
                    .col(big_integer(ChatConfig::TgId).primary_key())
                    .col(boolean(ChatConfig::CmdRandomEnabled).default(false))
                    .col(timestamp_with_time_zone(ChatConfig::UpdatedAt).default(Expr::current_timestamp()))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_chat_configs_chat_id")
                            .from(ChatConfig::Table, ChatConfig::TgId)
                            .to("chats", "tg_id")
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChatConfig::Table).if_exists().to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum ChatConfig {
    #[sea_orm(iden = "chat_configs")]
    Table,
    TgId,
    CmdRandomEnabled,
    UpdatedAt,
}
