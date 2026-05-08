use sea_orm_migration::{
    async_trait::async_trait,
    prelude::{extension::postgres::Type, *},
    schema::enumeration_null,
    sea_orm::{EnumIter, Iterable as _},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_type(Type::create().as_enum(ChatTypeEnum).values(ChatTypeVariants::iter()).to_owned())
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(Chat::Table)
                    .add_column(enumeration_null(Chat::ChatType, ChatTypeEnum, ChatTypeVariants::iter()).default(Expr::null()))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(Table::alter().table(Chat::Table).drop_column(Chat::ChatType).to_owned())
            .await?;

        manager
            .drop_type(Type::drop().name(ChatTypeEnum).if_exists().restrict().to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
#[sea_orm(iden = "chat_type")]
struct ChatTypeEnum;

#[derive(DeriveIden, EnumIter)]
enum ChatTypeVariants {
    #[sea_orm(iden = "private")]
    Private,
    #[sea_orm(iden = "group")]
    Group,
    #[sea_orm(iden = "supergroup")]
    Supergroup,
    #[sea_orm(iden = "channel")]
    Channel,
}

#[derive(DeriveIden)]
enum Chat {
    #[sea_orm(iden = "chats")]
    Table,
    ChatType,
}
