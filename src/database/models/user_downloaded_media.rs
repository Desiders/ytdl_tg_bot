use sea_orm::{
    ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EntityTrait as _, EnumIter, PrimaryKeyTrait, Related,
    RelationDef, RelationTrait as _,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{chat, downloaded_media};
use crate::entities::ChatDownloadedMedia;

#[derive(Debug, Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "chats_downloaded_media")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub chat_id: Uuid,
    pub user_downloaded_media: Uuid,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "chat::Entity",
        from = "Column::ChatId",
        to = "chat::Column::Id",
        fk_name = "fk_chats_downloaded_media_chats"
    )]
    Chat,
    #[sea_orm(
        belongs_to = "downloaded_media::Entity",
        from = "Column::UserDownloadedMedia",
        to = "downloaded_media::Column::Id",
        fk_name = "fk_chats_downloaded_media_media"
    )]
    DownloadedMedia,
}

impl Related<chat::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Chat.def()
    }
}

impl Related<downloaded_media::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::DownloadedMedia.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for ChatDownloadedMedia {
    fn from(
        Model {
            id,
            chat_id,
            user_downloaded_media,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            chat_id,
            user_downloaded_media,
            created_at,
        }
    }
}
