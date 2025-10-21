use sea_orm_migration::{
    async_trait::async_trait,
    prelude::{extension::postgres::Type, *},
    schema::*,
    sea_orm::{EnumIter, Iterable as _},
};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_type(Type::create().as_enum(MediaType).values(MediaTypeVariants::iter()).to_owned())
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Chat::Table)
                    .if_not_exists()
                    .col(big_integer(Chat::TgId).primary_key())
                    .col(string_null(Chat::Username).default(Keyword::Null))
                    .col(timestamp_with_time_zone(Chat::CreatedAt).default(Expr::current_timestamp()))
                    .col(timestamp_with_time_zone(Chat::UpdatedAt).default(Expr::current_timestamp()))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(DownloadedMedia::Table)
                    .if_not_exists()
                    .col(string(DownloadedMedia::FileId).primary_key())
                    .col(string(DownloadedMedia::UrlOrId))
                    .col(enumeration(DownloadedMedia::MediaType, MediaType, MediaTypeVariants::iter()))
                    .col(big_integer(DownloadedMedia::ChatTgId))
                    .col(timestamp_with_time_zone(DownloadedMedia::CreatedAt).default(Expr::current_timestamp()))
                    .foreign_key(
                        ForeignKey::create()
                            .from(DownloadedMedia::Table, DownloadedMedia::ChatTgId)
                            .to(Chat::Table, Chat::TgId)
                            .on_update(ForeignKeyAction::Cascade)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .index(
                        Index::create()
                            .name("idx_downloaded_media_url_media_type")
                            .col(DownloadedMedia::UrlOrId)
                            .col(DownloadedMedia::MediaType)
                            .unique(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DownloadedMedia::Table).if_exists().to_owned())
            .await?;
        manager.drop_table(Table::drop().table(Chat::Table).if_exists().to_owned()).await?;

        manager
            .drop_type(Type::drop().name(MediaType).if_exists().restrict().to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
struct MediaType;

#[derive(DeriveIden, EnumIter)]
pub enum MediaTypeVariants {
    Video,
    Audio,
}

#[derive(DeriveIden)]
enum Chat {
    #[sea_orm(iden = "chats")]
    Table,
    TgId,
    Username,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum DownloadedMedia {
    Table,
    FileId,
    UrlOrId,
    ChatTgId,
    MediaType,
    CreatedAt,
}
