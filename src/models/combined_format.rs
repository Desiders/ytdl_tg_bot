use super::{AudioFormat, FormatKind, VideoFormat};

use std::ops::Deref;

#[derive(Clone, Debug)]
pub struct CombinedFormat<'a> {
    pub video_format: VideoFormat<'a>,
    pub audio_format: AudioFormat<'a>,
}

impl<'a> CombinedFormat<'a> {
    pub fn new(video_format: VideoFormat<'a>, audio_format: AudioFormat<'a>) -> Self {
        Self {
            video_format,
            audio_format,
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

    pub fn format_id(&self) -> Box<str> {
        let video_format_id = self.video_format.id;
        let audio_format_id = self.audio_format.id;

        format!("{video_format_id}+{audio_format_id}").into_boxed_str()
    }

    pub const fn get_extension(&self) -> &str {
        self.video_format.container.as_str()
    }

    pub fn get_priority(&self) -> u8 {
        self.video_format.get_priority() + self.audio_format.get_priority()
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
            let a_priority = a.get_priority();
            let b_priority = b.get_priority();

            a_priority.cmp(&b_priority)
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
                FormatKind::CombinedFormat(audio_format, video_format) => {
                    combined_formats.push(CombinedFormat::new(video_format, audio_format));
                }
            }
        }

        for audio_format in &audio_formats {
            for video_format in &video_formats {
                if !audio_format
                    .codec
                    .is_support_container_with_vcodec(&video_format.codec, &video_format.container)
                {
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
