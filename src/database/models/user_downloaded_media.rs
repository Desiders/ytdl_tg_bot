use sea_orm::{
    ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EntityTrait as _, EnumIter, PrimaryKeyTrait, Related,
    RelationDef, RelationTrait as _,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{chat, downloaded_media, user};
use crate::entities::UserDownloadedMedia;

#[derive(Debug, Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users_downloaded_media")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub chat_id: Option<Uuid>,
    pub user_downloaded_media: Uuid,
    pub created_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "user::Entity",
        from = "Column::UserId",
        to = "user::Column::Id",
        fk_name = "fk_users_downloaded_media_users"
    )]
    User,
    #[sea_orm(
        belongs_to = "chat::Entity",
        from = "Column::ChatId",
        to = "chat::Column::Id",
        fk_name = "fk_users_downloaded_media_chats"
    )]
    Chat,
    #[sea_orm(
        belongs_to = "downloaded_media::Entity",
        from = "Column::UserDownloadedMedia",
        to = "downloaded_media::Column::Id",
        fk_name = "fk_users_downloaded_media"
    )]
    DownloadedMedia,
}

impl Related<user::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::User.def()
    }
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

impl From<Model> for UserDownloadedMedia {
    fn from(
        Model {
            id,
            user_id,
            chat_id,
            user_downloaded_media,
            created_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            user_id,
            chat_id,
            user_downloaded_media,
            created_at,
        }
    }
}
