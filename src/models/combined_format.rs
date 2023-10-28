use super::Format;
use crate::errors::FormatError;

use std::{collections::HashMap, ops::Deref};

const VIDEO_FORMATS: [&str; 26] = [
    "701", // 2160p60 + mp4
    "401", // 2160p60 + mp4
    "305", // 2160p60 + mp4
    "266", // 2160p30 + mp4
    "700", // 1440p60 + mp4
    "400", // 1440p60 + mp4
    "304", // 1440p60 + mp4
    "264", // 1440p30 + mp4
    "699", // 1080p60 + mp4
    "399", // 1080p60 + mp4
    "299", // 1080p60 + mp4
    "137", // 1080p30 + mp4
    "698", // 720p60 + mp4
    "398", // 720p60 + mp4
    "298", // 720p60 + mp4
    "136", // 720p30 + mp4
    "697", // 480p60 + mp4
    "397", // 480p60 + mp4
    "135", // 480p30 + mp4
    "696", // 360p60 + mp4
    "396", // 360p30 + mp4
    "134", // 360p30 + mp4
    "133", // 240p30 + mp4
    "395", // 240p30 + mp4
    "160", // 144p30 + mp4
    "394", // 144p30 + mp4
];

const AUDIO_FORMATS: [&str; 8] = [
    "258", // 386k + m4a
    "256", // 192k + m4a
    "251", // 160k + Opus
    "141", // 256k + m4a
    "140", // 128k + m4a
    "139", // 48k + m4a
    "250", // 70k + Opus
    "249", // 50k + Opus
];

const VIDEO_PLUS_AUDIO_FORMATS: [&str; 12] = [
    "301", // 1080p60 + mp4 + 128k + m4a
    "96",  // 1080p30 + mp4 + 256k + m4a
    "37",  // 1080p30 + mp4 + 128K + m4a
    "300", // 720p60 + mp4 + 128k + m4a
    "95",  // 720p30 + mp4 + 256k + m4a
    "22",  // 720p30 + mp4 + 128k + m4a
    "59",  // 480p30 + mp4 + 128k + m4a
    "94",  // 480p30 + mp4 + 128k + m4a
    "93",  // 360p30 + mp4 + 128k + m4a
    "18",  // 360p30 + mp4 + 96k + m4a
    "92",  // 240p + mp4 + 48k + m4a
    "91",  // 144p + mp4 + 48k + m4a
];

#[derive(Clone, Debug)]
pub struct CombinedFormat<'a> {
    pub video_format: &'a Format,
    pub audio_format: &'a Format,
}

impl<'a> CombinedFormat<'a> {
    pub fn new(video_format: &'a Format, audio_format: &'a Format) -> CombinedFormat<'a> {
        CombinedFormat {
            video_format,
            audio_format,
        }
    }

    pub fn get_format_id(&self) -> Option<String> {
        let Some(video_format) = self.video_format.format_id.as_ref() else {
            return None;
        };
        let Some(audio_format) = self.audio_format.format_id.as_ref() else {
            return None;
        };

        Some(format!("{video_format}+{audio_format}"))
    }
}

#[derive(Clone, Debug, Default)]
pub struct CombinedFormats<'a>(pub Vec<CombinedFormat<'a>>);

impl<'a> CombinedFormats<'a> {
    pub fn push(&mut self, combined_format: CombinedFormat<'a>) {
        self.0.push(combined_format);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'a> CombinedFormats<'a> {
    pub fn skip_with_size_less_than(&mut self, size: u64) {
        self.0.retain(|combined_format| {
            let video_format = combined_format.video_format;
            let audio_format = combined_format.audio_format;

            let video_format_size = video_format
                .filesize
                .as_ref()
                .unwrap_or(video_format.filesize_approx.as_ref().unwrap_or(&0.0));
            let audio_format_size = audio_format
                .filesize
                .as_ref()
                .unwrap_or(audio_format.filesize_approx.as_ref().unwrap_or(&0.0));

            (video_format_size + audio_format_size).round() as u64 <= size
        });
    }
}

impl<'a> Iterator for CombinedFormats<'a> {
    type Item = CombinedFormat<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop()
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

impl<'a> TryFrom<&'a [Format]> for CombinedFormats<'a> {
    type Error = FormatError<'a>;

    fn try_from(formats: &'a [Format]) -> Result<Self, Self::Error> {
        let mut formats_map = HashMap::with_capacity(formats.len());

        for format in formats {
            if let Some(format_id) = format.format_id.as_ref() {
                formats_map.insert(format_id.as_str(), format);
            } else {
                return Err(FormatError::FormatIdNotFound { format });
            }
        }

        let mut combined_formats = CombinedFormats::default();

        for video_format_id in VIDEO_FORMATS {
            if let Some(video_format) = formats_map.get(video_format_id) {
                for audio_format_id in AUDIO_FORMATS {
                    if let Some(audio_format) = formats_map.get(audio_format_id) {
                        combined_formats.push(CombinedFormat::new(video_format, audio_format));
                    }
                }
            }
        }

        for video_plus_audio_format_id in VIDEO_PLUS_AUDIO_FORMATS {
            if let Some(video_plus_audio_format) = formats_map.get(video_plus_audio_format_id) {
                combined_formats.push(CombinedFormat::new(video_plus_audio_format, video_plus_audio_format));
            }
        }

        Ok(combined_formats)
    }
}

impl<'a> TryFrom<Option<&'a [Format]>> for CombinedFormats<'a> {
    type Error = FormatError<'a>;

    fn try_from(formats: Option<&'a [Format]>) -> Result<Self, Self::Error> {
        match formats {
            Some(formats) => CombinedFormats::try_from(formats),
            None => Ok(CombinedFormats::default()),
        }
    }
}
