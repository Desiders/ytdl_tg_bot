pub use sea_orm_migration::prelude::*;

mod m20220101_000001_create_table;
mod m20251113_100712_remove_chat_from_downloaded_media;
mod m20260112_142259_chat_config;
mod m20260123_213835_add_downloaded_media_language;
mod m20260210_174551_add_downloaded_media_sections;
mod m20260217_205206_add_chat_config_exclude_domains;
mod m20260228_070901_add_chat_config_link_is_visible;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20220101_000001_create_table::Migration),
            Box::new(m20251113_100712_remove_chat_from_downloaded_media::Migration),
            Box::new(m20260112_142259_chat_config::Migration),
            Box::new(m20260123_213835_add_downloaded_media_language::Migration),
            Box::new(m20260210_174551_add_downloaded_media_sections::Migration),
            Box::new(m20260217_205206_add_chat_config_exclude_domains::Migration),
            Box::new(m20260228_070901_add_chat_config_link_is_visible::Migration),
        ]
    }
}
