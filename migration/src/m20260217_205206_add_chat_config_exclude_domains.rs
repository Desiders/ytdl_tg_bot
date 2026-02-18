use sea_orm_migration::{async_trait::async_trait, prelude::*, schema::*};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ChatConfigExcludeDomains::Table)
                    .if_not_exists()
                    .col(big_integer(ChatConfigExcludeDomains::TgId))
                    .col(string(ChatConfigExcludeDomains::Domain))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_chat_config_domains_chat_id")
                            .from(ChatConfigExcludeDomains::Table, ChatConfigExcludeDomains::TgId)
                            .to("chats", "tg_id")
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .index(
                        Index::create()
                            .name("idx_chat_config_domains_id_domain")
                            .col(ChatConfigExcludeDomains::TgId)
                            .col(ChatConfigExcludeDomains::Domain)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ChatConfigExcludeDomains::Table).if_exists().to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum ChatConfigExcludeDomains {
    #[sea_orm(iden = "chat_config_exclude_domains")]
    Table,
    TgId,
    Domain,
}
