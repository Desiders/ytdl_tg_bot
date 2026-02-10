use mutagen_rs::{
    common::error::MutagenError,
    id3::{
        frames::{Frame, PictureFrame},
        load_id3, save_id3,
        specs::{Encoding, PictureType},
    },
    mp4::{MP4Cover, MP4CoverFormat, MP4File, MP4TagValue},
};
use std::{io, path::Path};
use tracing::instrument;

#[derive(Debug, thiserror::Error)]
pub enum EmbedThumbnailErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Mutagen error: {0}")]
    Mutagen(#[from] MutagenError),
    #[error("Unsupported ext error: {0}")]
    UnsupportedExt(String),
}

#[instrument(skip_all)]
pub async fn embed_thumbnail(video_path: &Path, thumbnail_path: &Path) -> Result<(), EmbedThumbnailErrorKind> {
    let ext = video_path.extension().and_then(|val| val.to_str().map(str::to_lowercase));
    let video_path = video_path.to_string_lossy();

    match ext.as_deref() {
        Some("mp3") => {
            let (mut tags, _) = load_id3(&video_path)?;
            tags.add(Frame::Picture(PictureFrame {
                id: "APIC".to_owned(),
                encoding: Encoding::Utf8,
                mime: "image/jpeg".to_owned(),
                pic_type: PictureType::CoverFront,
                desc: "Cover".to_owned(),
                data: tokio::fs::read(thumbnail_path).await?,
            }));
            save_id3(&video_path, &tags, 3)?;
        }
        Some("mp4" | "m4a" | "mov") => {
            let mut file = MP4File::open(&video_path)?;
            file.tags.set(
                "covr",
                MP4TagValue::Cover(vec![MP4Cover {
                    data: tokio::fs::read(thumbnail_path).await?,
                    format: MP4CoverFormat::JPEG,
                }]),
            );
            file.save()?;
        }
        Some(ext) => return Err(EmbedThumbnailErrorKind::UnsupportedExt(ext.to_owned())),
        None => unreachable!("Video extension not found"),
    }

    Ok(())
}
