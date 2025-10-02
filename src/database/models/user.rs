use sea_orm::{ActiveModelBehavior, DeriveEntityModel, DerivePrimaryKey, DeriveRelation, EnumIter, PrimaryKeyTrait};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::entities::User;

#[derive(Debug, Clone, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: Uuid,
    pub tg_id: i64,
    pub username: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for User {
    fn from(
        Model {
            id,
            tg_id,
            username,
            created_at,
            updated_at,
        }: Model,
    ) -> Self {
        Self {
            id,
            tg_id,
            username: username.map(String::into_boxed_str),
            created_at,
            updated_at,
        }
    }
}
