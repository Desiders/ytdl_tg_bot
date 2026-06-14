use std::{io, path::Path};

use lofty::{
    config::WriteOptions,
    error::LoftyError,
    picture::{MimeType, Picture, PictureType},
    prelude::{AudioFile, TaggedFileExt},
    read_from_path,
    tag::Tag,
};
use tracing::instrument;

const COVER_DESCRIPTION: &str = "Cover";

#[derive(Debug, thiserror::Error)]
pub enum EmbedThumbnailErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Tagging error: {0}")]
    Lofty(#[from] LoftyError),
    #[error("Embed task panicked")]
    Join,
}

#[instrument(skip_all)]
pub async fn embed_thumbnail(media_path: &Path, thumbnail_path: &Path) -> Result<(), EmbedThumbnailErrorKind> {
    let thumbnail = tokio::fs::read(thumbnail_path).await?;
    let media_path = media_path.to_path_buf();

    // lofty buffers the whole media file in memory to rewrite it; run the blocking work off the
    // async runtime so a large write does not stall a worker thread.
    tokio::task::spawn_blocking(move || embed(&media_path, thumbnail))
        .await
        .map_err(|_| EmbedThumbnailErrorKind::Join)?
}

fn embed(media_path: &Path, thumbnail: Vec<u8>) -> Result<(), EmbedThumbnailErrorKind> {
    let mut tagged_file = read_from_path(media_path)?;

    let tag = match tagged_file.primary_tag_mut() {
        Some(tag) => tag,
        None => {
            let tag_type = tagged_file.primary_tag_type();
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.primary_tag_mut().expect("primary tag present after insert")
        }
    };

    let picture = Picture::unchecked(thumbnail)
        .pic_type(PictureType::CoverFront)
        .mime_type(MimeType::Jpeg)
        .description(COVER_DESCRIPTION)
        .build();

    while !tag.pictures().is_empty() {
        tag.remove_picture(0);
    }
    tag.push_picture(picture);

    tagged_file.save_to_path(media_path, WriteOptions::default())?;
    Ok(())
}
