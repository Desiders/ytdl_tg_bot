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

    #[must_use]
    pub fn get_vbr_plus_abr(&self) -> f32 {
        self.video_format.vbr.unwrap_or(0.0) + self.audio_format.abr.unwrap_or(0.0)
    }

    pub fn get_language(&self) -> Option<&str> {
        self.audio_format.language.or(self.video_format.language)
    }
}

impl Display for Format<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "audio: {}, video: {}", self.audio_format, self.video_format)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Formats<'a>(Vec<Format<'a>>);

impl<'a> Formats<'a> {
    pub fn push(&mut self, combined_format: Format<'a>) {
        self.0.push(combined_format);
    }
}

impl Formats<'_> {
    fn filter_by_max_size(&mut self, max_size: f64) {
        self.0
            .retain(|format| format.filesize_or_approx().is_none_or(|size| size <= max_size));
    }

    #[allow(clippy::unnecessary_cast, clippy::cast_possible_truncation, clippy::cast_precision_loss)]
    fn sort_formats(&mut self, max_size: f64, preferred_languages: &[Box<str>]) {
        fn calculate_size_weight(format: &Format, max_size: f64) -> f32 {
            match format.filesize_or_approx() {
                Some(size) => {
                    let distance = (max_size - size).abs();
                    if distance <= max_size * 0.2 {
                        (1.0 - (distance / (max_size * 0.2)).min(1.0) * 0.2) as f32
                    } else {
                        (0.5 - (((distance - max_size * 0.2) / (max_size * 0.8)).min(1.0)) * 0.2) as f32
                    }
                }
                None => 0.3,
            }
        }

        fn calculate_size_language(language: Option<&str>, preferred_languages: &[Box<str>]) -> f32 {
            let Some(language) = language else {
                return 0.2;
            };

            if let Some(pos) = preferred_languages
                .iter()
                .position(|preferred_language| preferred_language.eq_ignore_ascii_case(language))
            {
                return (1.0 - pos as f32 * 0.1).max(0.4);
            }

            0.0
        }

        let max_vbr = self
            .0
            .iter()
            .map(Format::get_vbr_plus_abr)
            .fold(0.0, |max, vbr| (max as f32).max(vbr));

        self.0.sort_by(|a, b| {
            let mut vbr_weight_a = a.get_vbr_plus_abr() / max_vbr;
            if vbr_weight_a.is_nan() {
                vbr_weight_a = 0.0;
            }
            let mut vbr_weight_b = b.get_vbr_plus_abr() / max_vbr;
            if vbr_weight_b.is_nan() {
                vbr_weight_b = 0.0;
            }

            let size_weight_a = calculate_size_weight(a, max_size);
            let size_weight_b = calculate_size_weight(b, max_size);

            let priority_weight_a = 1.0 / (f32::from(a.get_priority()) + 1.0);
            let priority_weight_b = 1.0 / (f32::from(b.get_priority()) + 1.0);

            let combined_bonus_a = if a.format_ids_are_equal() { 0.75 } else { 0.0 };
            let combined_bonus_b = if b.format_ids_are_equal() { 0.75 } else { 0.0 };

            let language_a = a.get_language();
            let language_b = b.get_language();

            let language_weight_a = calculate_size_language(language_a, preferred_languages);
            let language_weight_b = calculate_size_language(language_b, preferred_languages);

            let total_weight_a = vbr_weight_a + size_weight_a * 2.0 + priority_weight_a + combined_bonus_a + language_weight_a * 2.0;
            let total_weight_b = vbr_weight_b + size_weight_b * 2.0 + priority_weight_b + combined_bonus_b + language_weight_b * 2.0;

            total_weight_b.partial_cmp(&total_weight_a).unwrap_or(Ordering::Equal)
        });
    }

    pub fn sort(&mut self, max_size: u32, preferred_languages: &[Box<str>]) {
        let max_size = f64::from(max_size);

        self.filter_by_max_size(max_size);
        self.sort_formats(max_size, preferred_languages);
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
