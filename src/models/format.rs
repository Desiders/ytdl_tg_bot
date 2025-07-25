use crate::errors::FormatError;

use lazy_static::lazy_static;
use serde::{Deserialize, Deserializer};
use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
    ops::Deref,
};

const DEFAULT_PRIORITY: u8 = 19;
const DEFAULT_VIDEO_CODEC_PRIORITY: u8 = 6;

// Source: https://voussoir.net/writing/youtubedl_formats
lazy_static! {
    static ref VIDEO_IDS_AND_PRIORITY: HashMap<&'static str, u8> = HashMap::from([
        ("571", 1),
        ("272", 2),
        ("337", 3),
        ("401", 3),
        ("305", 3),
        ("315", 3),
        ("266", 4),
        ("313", 4),
        ("336", 5),
        ("400", 5),
        ("404", 5),
        ("304", 5),
        ("308", 5),
        ("264", 6),
        ("271", 6),
        ("335", 7),
        ("399", 7),
        ("299", 7),
        ("303", 7),
        ("137", 8),
        ("248", 8),
        ("334", 9),
        ("398", 9),
        ("298", 9),
        ("302", 9),
        ("136", 10),
        ("247", 10),
        ("333", 11),
        ("397", 12),
        ("135", 12),
        ("244", 12),
        ("332", 13),
        ("396", 14),
        ("134", 14),
        ("243", 14),
        ("331", 15),
        ("395", 16),
        ("133", 16),
        ("242", 16),
        ("330", 17),
        ("394", 18),
        ("160", 18),
        ("278", 18),
    ]);
    static ref AUDIO_IDS_AND_PRIORITY: HashMap<&'static str, u8> =
        HashMap::from([("258", 1), ("256", 2), ("251", 3), ("140", 4), ("250", 5), ("249", 6)]);
    static ref COMBINED_IDS_AND_PRIORITY: HashMap<&'static str, u8> = HashMap::from([("38", 2), ("37", 7), ("22", 14), ("18", 18)]);
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone)]
pub enum Container {
    MP4,
    MOV,
    MKV,
    TS,
}

fn is_mp4(container: &str) -> bool {
    container.to_lowercase().starts_with("mp4") || container.to_lowercase().starts_with("m4")
}

fn is_mov(container: &str) -> bool {
    container.to_lowercase().starts_with("mov") || container.to_lowercase().starts_with("qt")
}

fn is_mkv(container: &str) -> bool {
    container.to_lowercase().starts_with("mkv")
        || container.to_lowercase().starts_with("mk3d")
        || container.to_lowercase().starts_with("mka")
        || container.to_lowercase().starts_with("mks")
}

fn is_ts(container: &str) -> bool {
    container.to_lowercase().starts_with("ts")
        || container.to_lowercase().starts_with("tsv")
        || container.to_lowercase().starts_with("tsa")
        || container.to_lowercase().starts_with("m2t")
}

impl Container {
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::MP4 => "mp4",
            Self::MOV => "mov",
            Self::MKV => "mkv",
            Self::TS => "ts",
        }
    }

    #[must_use]
    pub const fn get_priority(&self) -> u8 {
        use Container::{MKV, MOV, MP4, TS};

        match self {
            MP4 => 1,
            MOV => 2,
            MKV => 3,
            TS => 4,
        }
    }
}

impl<'a> TryFrom<&'a str> for Container {
    type Error = FormatError<'a>;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        match value {
            _ if is_mp4(value) => Ok(Self::MP4),
            _ if is_mov(value) => Ok(Self::MOV),
            _ if is_mkv(value) => Ok(Self::MKV),
            _ if is_ts(value) => Ok(Self::TS),
            _ => Err(FormatError::ContainerNotSupported { container: value }),
        }
    }
}

impl<'a> TryFrom<(&AudioCodec<'a>, &VideoCodec<'a>)> for Container {
    type Error = FormatError<'a>;

    fn try_from(value: (&AudioCodec<'a>, &VideoCodec<'a>)) -> Result<Self, Self::Error> {
        let (audio_codec, video_codec) = value;

        if audio_codec.is_support_container_with_vcodec(video_codec, &Self::MP4) {
            return Ok(Self::MP4);
        }
        if audio_codec.is_support_container_with_vcodec(video_codec, &Self::MKV) {
            return Ok(Self::MKV);
        }
        if audio_codec.is_support_container_with_vcodec(video_codec, &Self::MOV) {
            return Ok(Self::MOV);
        }
        if audio_codec.is_support_container_with_vcodec(video_codec, &Self::TS) {
            return Ok(Self::TS);
        }

        Err(FormatError::VideoContainerEmpty)
    }
}

impl Display for Container {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone)]
pub enum VideoCodec<'a> {
    H265(&'a str),
    VP9(&'a str),
    H264(&'a str),
    ProRes(&'a str),
}

fn is_h264(codec: &str) -> bool {
    codec.to_lowercase().starts_with("avc")
        || codec.to_lowercase().starts_with("h264")
        || codec.to_lowercase().starts_with("avc1")
        || codec.to_lowercase().starts_with("avc3")
}

fn is_h265(codec: &str) -> bool {
    codec.to_lowercase().starts_with("hevc")
        || codec.to_lowercase().starts_with("h265")
        || codec.to_lowercase().starts_with("hev1")
        || codec.to_lowercase().starts_with("hvc1")
}

fn is_vp9(codec: &str) -> bool {
    codec.to_lowercase().starts_with("vp9")
        || codec.to_lowercase().starts_with("vp09")
        || codec.to_lowercase().starts_with("vp9x")
        || codec.to_lowercase().starts_with("vp09x")
}

fn is_prores(codec: &str) -> bool {
    codec.to_lowercase().starts_with("apch")
        || codec.to_lowercase().starts_with("apcn")
        || codec.to_lowercase().starts_with("apcs")
        || codec.to_lowercase().starts_with("apco")
        || codec.to_lowercase().starts_with("ap4h")
}

impl VideoCodec<'_> {
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::H264(codec) | Self::H265(codec) | Self::VP9(codec) | Self::ProRes(codec) => codec,
        }
    }

    #[must_use]
    pub const fn is_support_container(&self, container: &Container) -> bool {
        use Container::{MKV, MOV, MP4, TS};
        use VideoCodec::{ProRes, H264, H265, VP9};

        matches!(
            (self, container),
            (H264(_) | H265(_), MP4 | MOV | MKV | TS) | (VP9(_), MP4 | MKV) | (ProRes(_), MOV | MKV)
        )
    }

    #[must_use]
    pub const fn get_priority(&self) -> u8 {
        use VideoCodec::{ProRes, H264, H265, VP9};

        match self {
            H264(_) | H265(_) => 1,
            VP9(_) => 2,
            ProRes(_) => 4,
        }
    }

    #[must_use]
    pub const fn get_priority_by_container(&self, container: &Container) -> u8 {
        self.get_priority() + container.get_priority()
    }
}

impl<'a> TryFrom<&'a str> for VideoCodec<'a> {
    type Error = FormatError<'a>;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        match value {
            value if is_h264(value) => Ok(Self::H264(value)),
            value if is_h265(value) => Ok(Self::H265(value)),
            value if is_vp9(value) => Ok(Self::VP9(value)),
            value if is_prores(value) => Ok(Self::ProRes(value)),
            _ => Err(FormatError::VideoCodecNotSupported { codec: value }),
        }
    }
}

impl Display for VideoCodec<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[allow(clippy::upper_case_acronyms)]
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub enum AudioCodec<'a> {
    AAC_OR_ALAC(&'a str),
    FLAC(&'a str),
    Opus(&'a str),
    MP3(&'a str),
    PCM(&'a str),
}

fn is_aac_or_alac(codec: &str) -> bool {
    codec.to_lowercase().starts_with("aac")
        || codec.to_lowercase().starts_with("alac")
        || codec.to_lowercase().starts_with("m4a")
        || codec.to_lowercase().starts_with("m4p")
        || codec.to_lowercase().starts_with("m4b")
        || codec.to_lowercase().starts_with("mp4a")
        || codec.to_lowercase().starts_with("3gp")
}

fn is_flac(codec: &str) -> bool {
    codec.to_lowercase().starts_with("flac")
}

fn is_opus(codec: &str) -> bool {
    codec.to_lowercase().starts_with("opus")
}

fn is_mp3(codec: &str) -> bool {
    codec.to_lowercase().starts_with("mp3")
}

fn is_pcm(codec: &str) -> bool {
    codec.to_lowercase().starts_with("pcm")
}

impl AudioCodec<'_> {
    #[must_use]
    pub fn as_str(&self) -> String {
        match self {
            Self::AAC_OR_ALAC(codec) | Self::FLAC(codec) | Self::Opus(codec) | Self::MP3(codec) | Self::PCM(codec) => {
                format!("{extension} ({codec})", extension = self.get_extension())
            }
        }
    }

    #[must_use]
    pub const fn get_extension(&self) -> &str {
        match self {
            Self::AAC_OR_ALAC(_) => "m4a",
            Self::FLAC(_) => "flac",
            Self::Opus(_) => "opus",
            Self::MP3(_) => "mp3",
            Self::PCM(_) => "wav",
        }
    }

    #[must_use]
    pub const fn is_support_container_with_vcodec(&self, video_codec: &VideoCodec, container: &Container) -> bool {
        use AudioCodec::{Opus, AAC_OR_ALAC, FLAC, MP3, PCM};
        use Container::{MKV, MOV, MP4, TS};
        use VideoCodec::{ProRes, H264, H265, VP9};

        matches!(
            (self, video_codec, container),
            (AAC_OR_ALAC(_), H264(_) | H265(_), _)
                | (MP3(_), H264(_) | H265(_), MP4 | MOV | MKV | TS)
                | (AAC_OR_ALAC(_) | FLAC(_) | MP3(_) | Opus(_), VP9(_), MP4 | MKV)
                | (AAC_OR_ALAC(_) | PCM(_), ProRes(_), MOV | MKV)
                | (FLAC(_), H264(_) | H265(_), MP4 | MKV)
                | (FLAC(_) | Opus(_), ProRes(_), MKV)
                | (Opus(_), H264(_) | H265(_), MP4 | MKV | TS)
                | (PCM(_), H264(_) | H265(_), MOV | MKV)
                | (PCM(_), VP9(_), MKV)
        )
    }

    #[must_use]
    pub const fn get_priority(&self) -> u8 {
        use AudioCodec::{Opus, AAC_OR_ALAC, FLAC, MP3, PCM};

        match self {
            FLAC(_) => 1,
            AAC_OR_ALAC(_) => 2,
            Opus(_) => 3,
            MP3(_) => 4,
            PCM(_) => 5,
        }
    }
}

impl<'a> TryFrom<&'a str> for AudioCodec<'a> {
    type Error = FormatError<'a>;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        match value {
            value if is_aac_or_alac(value) => Ok(Self::AAC_OR_ALAC(value)),
            value if is_flac(value) => Ok(Self::FLAC(value)),
            value if is_opus(value) => Ok(Self::Opus(value)),
            value if is_mp3(value) => Ok(Self::MP3(value)),
            value if is_pcm(value) => Ok(Self::PCM(value)),
            _ => Err(FormatError::AudioCodecNotSupported { codec: value }),
        }
    }
}

impl Display for AudioCodec<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct Video<'a> {
    pub id: &'a str,
    pub url: &'a str,
    pub codec: Option<VideoCodec<'a>>,
    pub container: Container,
    pub vbr: Option<f32>,
    pub height: Option<f32>,
    pub width: Option<f32>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> Video<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: &'a str,
        url: &'a str,
        codec: Option<VideoCodec<'a>>,
        container: Container,
        vbr: Option<f32>,
        height: Option<f32>,
        width: Option<f32>,
        filesize: Option<f64>,
        filesize_approx: Option<f64>,
    ) -> Self {
        Self {
            id,
            url,
            codec,
            container,
            vbr,
            height,
            width,
            filesize,
            filesize_approx,
        }
    }

    pub fn get_priority(&self) -> u8 {
        let codec_priority = self.codec.as_ref().map_or(DEFAULT_VIDEO_CODEC_PRIORITY, |codec| {
            codec.get_priority_by_container(&self.container)
        });

        if let Some(priority) = VIDEO_IDS_AND_PRIORITY.get(self.id) {
            codec_priority + priority
        } else if let Some(priority) = COMBINED_IDS_AND_PRIORITY.get(self.id) {
            codec_priority + priority
        } else {
            codec_priority + DEFAULT_PRIORITY
        }
    }

    pub fn resolution(&self) -> String {
        let width = self.width.unwrap_or(0.0);
        let height = self.height.unwrap_or(0.0);

        format!("{width}x{height}")
    }

    pub fn filesize_or_approx(&self) -> Option<f64> {
        self.filesize.or(self.filesize_approx)
    }
}

impl Display for Video<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{id} {container}+{codec} {resolution} {filesize_kb:.2}KiB ({filesize_mb:.2}MiB)",
            id = self.id,
            container = self.container,
            codec = self.codec.as_ref().map_or("unknown", VideoCodec::as_str),
            resolution = self.resolution(),
            filesize_kb = self.filesize_or_approx().map_or(0.0, |filesize| filesize / 1024.0),
            filesize_mb = self.filesize_or_approx().map_or(0.0, |filesize| filesize / 1024.0 / 1024.0),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Audio<'a> {
    pub id: &'a str,
    pub url: &'a str,
    pub codec: AudioCodec<'a>,
    pub abr: Option<f32>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> Audio<'a> {
    pub fn new(
        id: &'a str,
        url: &'a str,
        codec: AudioCodec<'a>,
        abr: Option<f32>,
        filesize: Option<f64>,
        filesize_approx: Option<f64>,
    ) -> Self {
        Self {
            id,
            url,
            codec,
            abr,
            filesize,
            filesize_approx,
        }
    }

    pub fn get_priority(&self) -> u8 {
        let codec_priority = self.codec.get_priority();

        if let Some(priority) = AUDIO_IDS_AND_PRIORITY.get(self.id) {
            codec_priority + priority
        } else if let Some(priority) = COMBINED_IDS_AND_PRIORITY.get(self.id) {
            codec_priority + priority
        } else {
            codec_priority + DEFAULT_PRIORITY
        }
    }

    pub fn filesize_or_approx(&self) -> Option<f64> {
        self.filesize.or(self.filesize_approx)
    }
}

impl Display for Audio<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{id} {codec} {filesize_kb:.2}KB ({filesize_mb:.2}MB)",
            id = self.id,
            codec = self.codec,
            filesize_kb = self.filesize_or_approx().map_or(0.0, |filesize| filesize / 1024.0),
            filesize_mb = self.filesize_or_approx().map_or(0.0, |filesize| filesize / 1024.0 / 1024.0),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct Audios<'a>(pub Vec<Audio<'a>>);

impl Audios<'_> {
    fn skip_with_size_greater_than(&mut self, size: f64) {
        self.0.retain(|audio| {
            let Some(filesize) = audio.filesize else {
                return true;
            };

            filesize <= size
        });
    }

    fn sort_by_format_id_priority(&mut self) {
        self.0.sort_by_key(Audio::get_priority);
    }

    pub fn sort_by_priority_and_skip_by_size(&mut self, size: u32) {
        self.sort_by_format_id_priority();
        self.skip_with_size_greater_than(f64::from(size));
    }
}

impl<'a> Extend<Audio<'a>> for Audios<'a> {
    fn extend<T: IntoIterator<Item = Audio<'a>>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl<'a> Deref for Audios<'a> {
    type Target = Vec<Audio<'a>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> From<Vec<Audio<'a>>> for Audios<'a> {
    fn from(formats: Vec<Audio<'a>>) -> Self {
        Self(formats)
    }
}

impl Display for Audios<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (i, audio_format) in self.0.iter().enumerate() {
            if i != 0 {
                write!(f, ", ")?;
            }

            write!(f, "{audio_format}")?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum Kind<'a> {
    Audio(Audio<'a>),
    Video(Video<'a>),
    Combined(Audio<'a>, Video<'a>),
}

#[derive(Debug, Clone)]
enum Codec {
    None,
    Unknown,
    Inner(String),
}

impl Codec {
    #[must_use]
    pub const fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    #[must_use]
    pub const fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    #[must_use]
    pub const fn is_known(&self) -> bool {
        matches!(self, Self::Inner(_))
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::None | Self::Unknown => None,
            Self::Inner(codec) => Some(codec.as_str()),
        }
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct Any {
    pub id: String,
    pub url: String,
    pub ext: String,
    pub container: Option<String>,
    pub language: Option<String>,
    pub abr: Option<f32>,
    pub vbr: Option<f32>,
    pub tbr: Option<f32>,
    pub height: Option<f32>,
    pub width: Option<f32>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,

    acodec: Codec,
    vcodec: Codec,
}

impl Any {
    pub fn filesize_from_tbr(&self, duration: Option<f64>) -> Option<f64> {
        match (self.tbr, duration) {
            (Some(tbr), Some(duration)) => Some(duration * f64::from(tbr) * 1000.0 / 8.0),
            _ => None,
        }
    }
}

#[allow(clippy::similar_names)]
impl<'de> Deserialize<'de> for Any {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        struct Raw {
            format_id: String,
            url: String,
            ext: String,
            acodec: Option<String>,
            vcodec: Option<String>,
            container: Option<String>,
            language: Option<String>,
            abr: Option<f32>,
            vbr: Option<f32>,
            tbr: Option<f32>,
            height: Option<f32>,
            width: Option<f32>,
            filesize: Option<f64>,
            filesize_approx: Option<f64>,
        }

        let raw = Raw::deserialize(deserializer)?;

        let acodec = match raw.acodec {
            Some(acodec) => match acodec.as_str() {
                "none" => Codec::None,
                acodec => Codec::Inner(acodec.to_string()),
            },
            None => Codec::Unknown,
        };

        let vcodec = match raw.vcodec {
            Some(vcodec) => match vcodec.as_str() {
                "none" => Codec::None,
                vcodec => Codec::Inner(vcodec.to_string()),
            },
            None => Codec::Unknown,
        };

        Ok(Self {
            id: raw.format_id,
            url: raw.url,
            ext: raw.ext,
            acodec,
            vcodec,
            container: raw.container,
            language: raw.language,
            abr: raw.abr,
            vbr: raw.vbr,
            tbr: raw.tbr,
            height: raw.height,
            width: raw.width,
            filesize: raw.filesize,
            filesize_approx: raw.filesize_approx,
        })
    }
}

impl Any {
    #[allow(clippy::similar_names, clippy::too_many_lines)]
    pub fn kind(&self, duration: Option<f64>, preferred_languages: &[&str]) -> Result<Kind<'_>, FormatError<'_>> {
        if let Some(language) = &self.language {
            if !preferred_languages.contains(&language.as_str()) {
                return Err(FormatError::UnpreferredLanguage { language });
            }
        }

        let acodec = &self.acodec;
        let vcodec = &self.vcodec;

        if acodec.is_known() && vcodec.is_known() {
            let acodec = AudioCodec::try_from(acodec.as_str().unwrap())?;
            let vcodec = VideoCodec::try_from(vcodec.as_str().unwrap())?;

            let container = Container::try_from((&acodec, &vcodec))?;

            if !vcodec.is_support_container(&container) {
                return Err(FormatError::ContainerNotSupportedByVideoCodec {
                    container: container.as_str().to_string().into_boxed_str(),
                    codec: vcodec.as_str().to_string().into_boxed_str(),
                });
            }

            let audio_format = Audio::new(
                self.id.as_str(),
                self.url.as_str(),
                acodec,
                self.abr,
                self.filesize,
                self.filesize_approx.or(self.filesize_from_tbr(duration)),
            );

            let video_format = Video::new(
                self.id.as_str(),
                self.url.as_str(),
                Some(vcodec),
                container,
                self.vbr,
                self.height,
                self.width,
                self.filesize,
                self.filesize_approx.or(self.filesize_from_tbr(duration)),
            );

            Ok(Kind::Combined(audio_format, video_format))
        } else if (acodec.is_unknown() && vcodec.is_unknown()) | (acodec.is_none() && vcodec.is_none()) {
            let container = Container::try_from(self.ext.as_str())?;

            let audio_format = Audio::new(
                self.id.as_str(),
                self.url.as_str(),
                AudioCodec::try_from("mp3")?,
                self.abr,
                self.filesize,
                self.filesize_approx.or(self.filesize_from_tbr(duration)),
            );

            let video_format = Video::new(
                self.id.as_str(),
                self.url.as_str(),
                None,
                container,
                self.height,
                self.vbr,
                self.width,
                self.filesize,
                self.filesize_approx.or(self.filesize_from_tbr(duration)),
            );

            Ok(Kind::Combined(audio_format, video_format))
        } else if acodec.is_known() {
            let acodec = AudioCodec::try_from(acodec.as_str().unwrap())?;
            let audio_format = Audio::new(
                self.id.as_str(),
                self.url.as_str(),
                acodec,
                self.abr,
                self.filesize,
                self.filesize_approx.or(self.filesize_from_tbr(duration)),
            );

            Ok(Kind::Audio(audio_format))
        } else if vcodec.is_known() {
            let Some(container) = self.container.as_ref() else {
                return Err(FormatError::VideoContainerEmpty);
            };

            let vcodec = VideoCodec::try_from(vcodec.as_str().unwrap())?;
            let container = Container::try_from(container.as_str())?;

            if !vcodec.is_support_container(&container) {
                return Err(FormatError::ContainerNotSupportedByVideoCodec {
                    container: container.as_str().to_string().into_boxed_str(),
                    codec: vcodec.as_str().to_string().into_boxed_str(),
                });
            }

            let video_format = Video::new(
                self.id.as_str(),
                self.url.as_str(),
                Some(vcodec),
                container,
                self.vbr,
                self.height,
                self.width,
                self.filesize,
                self.filesize_approx.or(self.filesize_from_tbr(duration)),
            );

            Ok(Kind::Video(video_format))
        } else if acodec.is_unknown() {
            match AudioCodec::try_from(self.ext.as_str()) {
                Ok(acodec) => {
                    let audio_format = Audio::new(
                        self.id.as_str(),
                        self.url.as_str(),
                        acodec,
                        self.abr,
                        self.filesize,
                        self.filesize_approx.or(self.filesize_from_tbr(duration)),
                    );

                    Ok(Kind::Audio(audio_format))
                }
                Err(error) => Err(error),
            }
        } else if vcodec.is_unknown() {
            match Container::try_from(self.ext.as_str()) {
                Ok(container) => {
                    let video_format = Video::new(
                        self.id.as_str(),
                        self.url.as_str(),
                        None,
                        container,
                        self.vbr,
                        self.height,
                        self.width,
                        self.filesize,
                        self.filesize_approx.or(self.filesize_from_tbr(duration)),
                    );

                    Ok(Kind::Video(video_format))
                }
                Err(error) => Err(error),
            }
        } else {
            Err(FormatError::UnknownFormat)
        }
    }
}
