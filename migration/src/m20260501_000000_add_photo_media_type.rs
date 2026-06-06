use sea_orm_migration::{async_trait::async_trait, prelude::*, sea_query::extension::postgres::Type};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_type(Type::alter().name(MediaType).add_value(MediaTypeVariants::Photo).clone())
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}

#[derive(DeriveIden)]
struct MediaType;

#[derive(DeriveIden)]
enum MediaTypeVariants {
    Photo,
}
