use crate::errors::FormatError;

use lazy_static::lazy_static;
use serde::{Deserialize, Deserializer};
use std::{
    collections::HashMap,
    fmt::{self, Display, Formatter},
    ops::Deref,
};

const DEFAULT_PRIORITY: u8 = 19;

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
    static ref COMBINED_IDS_AND_PRIORITY: HashMap<&'static str, u8> = HashMap::from([("22", 14), ("18", 18)]);
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
    AV1(&'a str),
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

fn is_av1(codec: &str) -> bool {
    codec.to_lowercase().starts_with("av1")
        || codec.to_lowercase().starts_with("aom")
        || codec.to_lowercase().starts_with("av01")
        || codec.to_lowercase().starts_with("avo1")
        || codec.to_lowercase().starts_with("av1x")
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
            Self::H264(codec) | Self::H265(codec) | Self::AV1(codec) | Self::VP9(codec) | Self::ProRes(codec) => codec,
        }
    }

    #[must_use]
    pub const fn is_support_container(&self, container: &Container) -> bool {
        use Container::{MKV, MOV, MP4, TS};
        use VideoCodec::{ProRes, AV1, H264, H265, VP9};

        matches!(
            (self, container),
            (H264(_) | H265(_), MP4 | MOV | MKV | TS) | (AV1(_) | VP9(_), MP4 | MKV) | (ProRes(_), MOV | MKV)
        )
    }

    #[must_use]
    pub const fn get_priority(&self) -> u8 {
        use VideoCodec::{ProRes, AV1, H264, H265, VP9};

        match self {
            AV1(_) => 1,
            H265(_) => 2,
            VP9(_) => 3,
            H264(_) => 4,
            ProRes(_) => 5,
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
            value if is_av1(value) => Ok(Self::AV1(value)),
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
    PCM(&'a str),
}

fn is_aac_or_alac(codec: &str) -> bool {
    codec.to_lowercase().starts_with("aac")
        || codec.to_lowercase().starts_with("alac")
        || codec.to_lowercase().starts_with("m4a")
        || codec.to_lowercase().starts_with("m4p")
        || codec.to_lowercase().starts_with("m4b")
        || codec.to_lowercase().starts_with("mp4")
        || codec.to_lowercase().starts_with("3gp")
}

fn is_flac(codec: &str) -> bool {
    codec.to_lowercase().starts_with("flac")
}

fn is_opus(codec: &str) -> bool {
    codec.to_lowercase().starts_with("opus")
}

fn is_pcm(codec: &str) -> bool {
    codec.to_lowercase().starts_with("pcm")
}

impl AudioCodec<'_> {
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::AAC_OR_ALAC(codec) | Self::FLAC(codec) | Self::Opus(codec) | Self::PCM(codec) => codec,
        }
    }

    #[must_use]
    pub const fn get_extension(&self) -> &str {
        match self {
            Self::AAC_OR_ALAC(_) => "m4a",
            Self::FLAC(_) => "flac",
            Self::Opus(_) => "opus",
            Self::PCM(_) => "wav",
        }
    }

    #[must_use]
    pub const fn is_support_container_with_vcodec(&self, video_codec: &VideoCodec, container: &Container) -> bool {
        use AudioCodec::{Opus, AAC_OR_ALAC, FLAC, PCM};
        use Container::{MKV, MOV, MP4, TS};
        use VideoCodec::{ProRes, AV1, H264, H265, VP9};

        matches!(
            (self, video_codec, container),
            (AAC_OR_ALAC(_), H264(_), _)
                | (AAC_OR_ALAC(_), H265(_), MP4 | MOV | MKV | TS)
                | (AAC_OR_ALAC(_) | FLAC(_) | Opus(_), AV1(_) | VP9(_), MP4 | MKV)
                | (AAC_OR_ALAC(_) | PCM(_), ProRes(_), MOV | MKV)
                | (FLAC(_), H264(_) | H265(_), MP4 | MKV)
                | (FLAC(_) | Opus(_), ProRes(_), MKV)
                | (Opus(_), H264(_) | H265(_), MP4 | MKV | TS)
                | (PCM(_), H264(_) | H265(_), MOV | MKV)
                | (PCM(_), AV1(_) | VP9(_), MKV)
        )
    }

    #[must_use]
    pub const fn get_priority(&self) -> u8 {
        use AudioCodec::{Opus, AAC_OR_ALAC, FLAC, PCM};

        match self {
            FLAC(_) => 1,
            AAC_OR_ALAC(_) => 2,
            Opus(_) => 3,
            PCM(_) => 4,
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
    pub codec: VideoCodec<'a>,
    pub container: Container,
    pub height: Option<f64>,
    pub width: Option<f64>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> Video<'a> {
    pub fn new(
        id: &'a str,
        codec: VideoCodec<'a>,
        container: Container,
        height: Option<f64>,
        width: Option<f64>,
        filesize: Option<f64>,
        filesize_approx: Option<f64>,
    ) -> Self {
        Self {
            id,
            codec,
            container,
            height,
            width,
            filesize,
            filesize_approx,
        }
    }

    pub fn get_priority(&self) -> u8 {
        let codec_priority = self.codec.get_priority_by_container(&self.container);

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
            "{id}id {container}({codec}, {resolution}) {filesize_kb:.2}KB ({filesize_mb:.2}MB)",
            id = self.id,
            container = self.container,
            codec = self.codec,
            resolution = self.resolution(),
            filesize_kb = self.filesize_or_approx().map_or(0.0, |filesize| filesize.round() / 1024.0),
            filesize_mb = self.filesize_or_approx().map_or(0.0, |filesize| filesize.round() / 1024.0 / 1024.0),
        )
    }
}

#[derive(Debug, Clone)]
pub struct Audio<'a> {
    pub id: &'a str,
    pub codec: AudioCodec<'a>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> Audio<'a> {
    pub fn new(id: &'a str, codec: AudioCodec<'a>, filesize: Option<f64>, filesize_approx: Option<f64>) -> Self {
        Self {
            id,
            codec,
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
            "{id}id ({codec}) {filesize_kb:.2}KB ({filesize_mb:.2}MB)",
            id = self.id,
            codec = self.codec,
            filesize_kb = self.filesize_or_approx().map_or(0.0, |filesize| filesize.round() / 1024.0),
            filesize_mb = self.filesize_or_approx().map_or(0.0, |filesize| filesize.round() / 1024.0 / 1024.0),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct Audios<'a>(pub Vec<Audio<'a>>);

impl<'a> Audios<'a> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn skip_with_size_less_than(&mut self, size: u64) {
        self.0.retain(|audio| {
            let Some(filesize) = audio.filesize else {
                return false;
            };

            filesize.round() as u64 <= size
        });
    }

    pub fn sort_by_format_id_priority(&mut self) {
        self.0.sort_by_key(Audio::get_priority);
    }

    pub fn sort_by_priority_and_skip_by_size(&mut self, size: u64) {
        self.sort_by_format_id_priority();
        self.skip_with_size_less_than(size);
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

#[derive(Debug, Clone)]
pub enum Kind<'a> {
    Audio(Audio<'a>),
    Video(Video<'a>),
    Combined(Audio<'a>, Video<'a>),
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct Any {
    pub id: String,
    pub acodec: Option<String>,
    pub vcodec: Option<String>,
    pub container: Option<String>,
    pub height: Option<f64>,
    pub width: Option<f64>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'de> Deserialize<'de> for Any {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            format_id: String,
            acodec: Option<String>,
            vcodec: Option<String>,
            container: Option<String>,
            height: Option<f64>,
            width: Option<f64>,
            filesize: Option<f64>,
            filesize_approx: Option<f64>,
        }

        let raw = Raw::deserialize(deserializer)?;

        Ok(Self {
            id: raw.format_id,
            acodec: raw.acodec.filter(|acodec| acodec != "none"),
            vcodec: raw.vcodec.filter(|vcodec| vcodec != "none"),
            container: raw.container,
            height: raw.height,
            width: raw.width,
            filesize: raw.filesize,
            filesize_approx: raw.filesize_approx,
        })
    }
}

impl Any {
    #[allow(clippy::similar_names)]
    pub fn kind(&self) -> Result<Kind<'_>, FormatError<'_>> {
        let acodec = self.acodec.as_ref();
        let vcodec = self.vcodec.as_ref();

        let is_combined = acodec.is_some() && vcodec.is_some();

        if is_combined {
            let acodec = AudioCodec::try_from(acodec.unwrap().as_str())?;
            let vcodec = VideoCodec::try_from(vcodec.unwrap().as_str())?;

            let container = Container::try_from((&acodec, &vcodec))?;

            if !vcodec.is_support_container(&container) {
                return Err(FormatError::ContainerNotSupportedByVideoCodec {
                    container: container.as_str().to_string().into_boxed_str(),
                    codec: vcodec.as_str().to_string().into_boxed_str(),
                });
            }

            let audio_format = Audio::new(self.id.as_str(), acodec, self.filesize, self.filesize_approx);

            let video_format = Video::new(
                self.id.as_str(),
                vcodec,
                container,
                self.height,
                self.width,
                self.filesize,
                self.filesize_approx,
            );

            return Ok(Kind::Combined(audio_format, video_format));
        }

        if let Some(acodec) = acodec {
            let acodec = AudioCodec::try_from(acodec.as_str())?;
            let audio_format = Audio::new(self.id.as_str(), acodec, self.filesize, self.filesize_approx);

            Ok(Kind::Audio(audio_format))
        } else if let Some(vcodec) = vcodec {
            let Some(container) = self.container.as_ref() else {
                return Err(FormatError::VideoContainerEmpty);
            };

            let vcodec = VideoCodec::try_from(vcodec.as_str())?;
            let container = Container::try_from(container.as_str())?;

            if !vcodec.is_support_container(&container) {
                return Err(FormatError::ContainerNotSupportedByVideoCodec {
                    container: container.as_str().to_string().into_boxed_str(),
                    codec: vcodec.as_str().to_string().into_boxed_str(),
                });
            }

            let video_format = Video::new(
                self.id.as_str(),
                vcodec,
                container,
                self.height,
                self.width,
                self.filesize,
                self.filesize_approx,
            );

            Ok(Kind::Video(video_format))
        } else {
            Err(FormatError::AudioAndVideoCodecsEmpty)
        }
    }
}

impl<'a> TryFrom<&'a Any> for Kind<'a> {
    type Error = FormatError<'a>;

    fn try_from(value: &'a Any) -> Result<Self, Self::Error> {
        value.kind()
    }
}
