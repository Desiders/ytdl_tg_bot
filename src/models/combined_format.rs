use super::{AudioFormat, FormatKind, VideoFormat};

use std::{cmp::Ordering, ops::Deref};

#[derive(Clone, Debug)]
pub struct CombinedFormat<'a> {
    video_format: VideoFormat<'a>,
    audio_format: AudioFormat<'a>,
    format_id: Box<str>,
}

impl<'a> CombinedFormat<'a> {
    pub fn new(video_format: VideoFormat<'a>, audio_format: AudioFormat<'a>) -> Self {
        let format_id = format!("{}+{}", video_format.id, audio_format.id).into_boxed_str();

        Self {
            video_format,
            audio_format,
            format_id,
        }
    }

    pub fn filesize(&self) -> Option<f64> {
        self.video_format
            .filesize
            .and_then(|video_filesize| self.audio_format.filesize.map(|audio_filesize| video_filesize + audio_filesize))
    }

    pub fn filesize_approx(&self) -> Option<f64> {
        self.video_format.filesize_approx.and_then(|video_filesize_approx| {
            self.audio_format
                .filesize_approx
                .map(|audio_filesize_approx| video_filesize_approx + audio_filesize_approx)
        })
    }

    pub fn filesize_or_approx(&self) -> Option<f64> {
        self.filesize().or(self.filesize_approx())
    }

    pub fn get_extension(&self) -> &str {
        self.video_format.get_extension()
    }

    pub const fn format_id(&self) -> &str {
        &self.format_id
    }
}

#[derive(Clone, Debug, Default)]
pub struct CombinedFormats<'a>(pub Vec<CombinedFormat<'a>>);

impl<'a> CombinedFormats<'a> {
    pub fn push(&mut self, combined_format: CombinedFormat<'a>) {
        self.0.push(combined_format);
    }
}

impl<'a> CombinedFormats<'a> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn skip_with_size_less_than(&mut self, size: u64) {
        self.0.retain(|combined_format| {
            let Some(filesize_or_approx) = combined_format.filesize_or_approx() else {
                return true;
            };

            filesize_or_approx.round() as u64 <= size
        });
    }

    pub fn sort_by_format_id_priority(&mut self) {
        self.0.sort_by(|a, b| {
            let a_video_format_id_priority = a.video_format.priority;
            let b_video_format_id_priority = b.video_format.priority;

            match a_video_format_id_priority.cmp(&b_video_format_id_priority) {
                Ordering::Equal => {
                    let a_audio_format_id_priority = a.audio_format.priority;
                    let b_audio_format_id_priority = b.audio_format.priority;

                    a_audio_format_id_priority.cmp(&b_audio_format_id_priority)
                }
                ordering => ordering,
            }
        });
    }
}

impl<'a> Extend<CombinedFormat<'a>> for CombinedFormats<'a> {
    fn extend<T: IntoIterator<Item = CombinedFormat<'a>>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl<'a> Deref for CombinedFormats<'a> {
    type Target = Vec<CombinedFormat<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> From<Vec<FormatKind<'a>>> for CombinedFormats<'a> {
    fn from(formats: Vec<FormatKind<'a>>) -> Self {
        let mut combined_formats = CombinedFormats::default();

        let mut video_formats = Vec::new();
        let mut audio_formats = Vec::new();

        for format in formats {
            match format {
                FormatKind::Video(video_format) => {
                    video_formats.push(video_format);
                }
                FormatKind::Audio(audio_format) => {
                    audio_formats.push(audio_format);
                }
            }
        }

        for audio_format in &audio_formats {
            for video_format in &video_formats {
                if !audio_format.support_video_format(video_format) {
                    continue;
                }

                let combined_format = CombinedFormat::new(video_format.clone(), audio_format.clone());

                combined_formats.push(combined_format);
            }
        }

        combined_formats
    }
}

impl<'a> From<Option<Vec<FormatKind<'a>>>> for CombinedFormats<'a> {
    fn from(formats: Option<Vec<FormatKind<'a>>>) -> Self {
        match formats {
            Some(formats) => CombinedFormats::from(formats),
            None => CombinedFormats::default(),
        }
    }
}
