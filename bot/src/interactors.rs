mod base;
#[cfg(test)]
mod download_tests;
#[cfg(test)]
mod menu_tests;

pub mod audio;
pub mod auto;
pub mod chosen_inline;
pub mod config;
pub mod enqueue_download;
pub mod inline_query;
pub mod lang;
pub mod menu;
pub mod photo;
pub mod shazam;
pub mod start;
pub mod stats;
pub mod video;

pub(super) use base::Interactor;
