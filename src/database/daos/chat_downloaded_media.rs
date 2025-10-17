use std::convert::Infallible;

use sea_orm::{ActiveValue::Set, ConnectionTrait, EntityTrait as _};

use crate::{database::models::chat_downloaded_media, entities::ChatDownloadedMedia, errors::database::ErrorKind};

pub struct Dao<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> Dao<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self
    where
        Conn: ConnectionTrait,
    {
        Self { conn }
    }
}

impl<Conn> Dao<'_, Conn>
where
    Conn: ConnectionTrait,
{
    pub async fn insert(
        &self,
        ChatDownloadedMedia {
            id,
            chat_id,
            downloaded_media,
            created_at,
        }: ChatDownloadedMedia,
    ) -> Result<ChatDownloadedMedia, ErrorKind<Infallible>> {
        use chat_downloaded_media::{ActiveModel, Entity};

        let model = ActiveModel {
            id: Set(id),
            chat_id: Set(chat_id),
            downloaded_media: Set(downloaded_media),
            created_at: Set(created_at),
        };

        Entity::insert(model)
            .exec_with_returning(self.conn)
            .await
            .map(Into::into)
            .map_err(Into::into)
    }
}
