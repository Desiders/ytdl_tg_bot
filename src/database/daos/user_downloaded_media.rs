use std::convert::Infallible;

use sea_orm::{ActiveValue::Set, ConnectionTrait, EntityTrait as _};

use crate::{database::models::user_downloaded_media, entities::UserDownloadedMedia, errors::database::ErrorKind};

pub struct UserDownloadedMediaDao<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> UserDownloadedMediaDao<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self
    where
        Conn: ConnectionTrait,
    {
        Self { conn }
    }
}

impl<Conn> UserDownloadedMediaDao<'_, Conn>
where
    Conn: ConnectionTrait,
{
    pub async fn insert(
        &self,
        UserDownloadedMedia {
            id,
            user_id,
            chat_id,
            user_downloaded_media,
            created_at,
        }: UserDownloadedMedia,
    ) -> Result<UserDownloadedMedia, ErrorKind<Infallible>> {
        use user_downloaded_media::{ActiveModel, Entity};

        let model = ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            chat_id: Set(chat_id),
            user_downloaded_media: Set(user_downloaded_media),
            created_at: Set(created_at),
        };

        Entity::insert(model)
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
