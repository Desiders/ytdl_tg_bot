use super::format;

use std::cmp::Ordering;
use std::{
    fmt::{self, Display, Formatter},
    ops::Deref,
};

#[derive(Clone, Debug)]
pub struct Format<'a> {
    pub video_format: format::Video<'a>,
    pub audio_format: format::Audio<'a>,
}

impl<'a> Format<'a> {
    #[must_use]
    pub fn new(video_format: format::Video<'a>, audio_format: format::Audio<'a>) -> Self {
        Self {
            video_format,
            audio_format,
        }
    }

    #[must_use]
    pub fn filesize(&self) -> Option<f64> {
        let video_filesize = self.video_format.filesize;
        let audio_filesize = self.audio_format.filesize;

        match (video_filesize, audio_filesize) {
            (Some(video_filesize), Some(audio_filesize)) => Some(video_filesize + audio_filesize),
            (Some(video_filesize), None) => Some(video_filesize),
            (None, Some(audio_filesize)) => Some(audio_filesize),
            (None, None) => None,
        }
    }

    #[must_use]
    pub fn filesize_approx(&self) -> Option<f64> {
        let video_filesize_approx = self.video_format.filesize_approx;
        let audio_filesize_approx = self.audio_format.filesize_approx;

        match (video_filesize_approx, audio_filesize_approx) {
            (Some(video_filesize_approx), Some(audio_filesize_approx)) => Some(video_filesize_approx + audio_filesize_approx),
            (Some(video_filesize_approx), None) => Some(video_filesize_approx),
            (None, Some(audio_filesize_approx)) => Some(audio_filesize_approx),
            (None, None) => None,
        }
    }

    #[must_use]
    pub fn filesize_or_approx(&self) -> Option<f64> {
        self.filesize().or(self.filesize_approx())
    }

    #[must_use]
    pub fn format_id(&self) -> Box<str> {
        let video_format_id = self.video_format.id;
        let audio_format_id = self.audio_format.id;

        format!("{video_format_id}+{audio_format_id}").into_boxed_str()
    }

    #[must_use]
    pub fn format_ids_are_equal(&self) -> bool {
        self.video_format.id == self.audio_format.id
    }

    #[must_use]
    pub const fn get_extension(&self) -> &str {
        self.video_format.container.as_str()
    }

    #[must_use]
    pub fn get_priority(&self) -> u8 {
        self.video_format.get_priority() + self.audio_format.get_priority()
    }

    pub fn get_vbr_plus_abr(&self) -> f64 {
        self.video_format.vbr.unwrap_or(0.0) + self.audio_format.abr.unwrap_or(0.0)
    }
}

impl Display for Format<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "audio: {}, video: {}", self.audio_format, self.video_format)
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
    pub fn filter_by_max_size(&mut self, max_size: u64) {
        self.0.retain(|format| {
            format
                .filesize_or_approx()
                .map(|size| size.round() as u64 <= max_size)
                .unwrap_or(true)
        });
    }

    pub fn sort_formats(&mut self, max_size: u64) {
        let max_vbr = self
            .0
            .iter()
            .map(|format| format.get_vbr_plus_abr())
            .fold(0.0, |max, vbr| (max as f32).max(vbr as f32)) as f64;

        self.0.sort_by(|a, b| {
            let vbr_weight_a = a.get_vbr_plus_abr() / max_vbr;
            let vbr_weight_b = b.get_vbr_plus_abr() / max_vbr;

            let size_weight_a = match a.filesize_or_approx() {
                Some(size) => {
                    let distance = (max_size as f64 - size).abs();
                    if distance <= max_size as f64 * 0.2 {
                        1.0
                    } else {
                        0.5
                    }
                }
                None => 0.3,
            };
            let size_weight_b = match b.filesize_or_approx() {
                Some(size) => {
                    let distance = (max_size as f64 - size).abs();
                    if distance <= max_size as f64 * 0.2 {
                        1.0
                    } else {
                        0.5
                    }
                }
                None => 0.3,
            };

            let priority_weight_a = 1.0 / (a.get_priority() as f64 + 1.0);
            let priority_weight_b = 1.0 / (b.get_priority() as f64 + 1.0);

            let total_weight_a = vbr_weight_a + size_weight_a * 2.0 + priority_weight_a;
            let total_weight_b = vbr_weight_b + size_weight_b * 2.0 + priority_weight_b;

            total_weight_b.partial_cmp(&total_weight_a).unwrap_or(Ordering::Equal)
        });
    }

    pub fn sort(&mut self, max_size: u64) {
        self.filter_by_max_size(max_size);
        self.sort_formats(max_size);
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
                if let Some(vcodec) = &video_format.codec {
                    if !audio_format.codec.is_support_container_with_vcodec(vcodec, &video_format.container) {
                        continue;
                    }
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

impl Display for Formats<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (i, combined_format) in self.0.iter().enumerate() {
            if i != 0 {
                write!(f, ", ")?;
            }

            write!(f, "{combined_format}")?;
        }

        Ok(())
    }
}
