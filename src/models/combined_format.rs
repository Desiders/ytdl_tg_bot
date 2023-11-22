use super::format;

use std::ops::Deref;

#[derive(Clone, Debug)]
pub struct Format<'a> {
    pub video_format: format::Video<'a>,
    pub audio_format: format::Audio<'a>,
}

impl<'a> Format<'a> {
    pub fn new(video_format: format::Video<'a>, audio_format: format::Audio<'a>) -> Self {
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
pub struct Formats<'a>(pub Vec<Format<'a>>);

impl<'a> Formats<'a> {
    pub fn push(&mut self, combined_format: Format<'a>) {
        self.0.push(combined_format);
    }
}

impl<'a> Formats<'a> {
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

    pub fn sort_by_priority_and_skip_by_size(&mut self, size: u64) {
        self.skip_with_size_less_than(size);
        self.sort_by_format_id_priority();
    }
}

impl<'a> Extend<Format<'a>> for Formats<'a> {
    fn extend<T: IntoIterator<Item = Format<'a>>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl<'a> Deref for Formats<'a> {
    type Target = Vec<Format<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> From<Vec<format::Kind<'a>>> for Formats<'a> {
    fn from(formats: Vec<format::Kind<'a>>) -> Self {
        let mut combined_formats = Formats::default();

        let mut video_formats = Vec::new();
        let mut audio_formats = Vec::new();

        for format in formats {
            match format {
                format::Kind::Video(video_format) => {
                    video_formats.push(video_format);
                }
                format::Kind::Audio(audio_format) => {
                    audio_formats.push(audio_format);
                }
                format::Kind::Combined(audio_format, video_format) => {
                    combined_formats.push(Format::new(video_format, audio_format));
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

                let combined_format = Format::new(video_format.clone(), audio_format.clone());

                combined_formats.push(combined_format);
            }
        }

        combined_formats
    }
}

impl<'a> From<Option<Vec<format::Kind<'a>>>> for Formats<'a> {
    fn from(formats: Option<Vec<format::Kind<'a>>>) -> Self {
        match formats {
            Some(formats) => Formats::from(formats),
            None => Formats::default(),
        }
    }
}
