use crate::errors::FormatError;

use lazy_static::lazy_static;
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;

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
            Self::H264(_) => "h264",
            Self::H265(_) => "h265",
            Self::AV1(_) => "av1",
            Self::VP9(_) => "vp9",
            Self::ProRes(_) => "prores",
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

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone)]
pub enum AudioCodec<'a> {
    AAC(&'a str),
    ALAC(&'a str),
    FLAC(&'a str),
    Opus(&'a str),
    PCM(&'a str),
}

fn is_aac(codec: &str) -> bool {
    codec.to_lowercase().starts_with("aac")
        || codec.to_lowercase().starts_with("m4a")
        || codec.to_lowercase().starts_with("m4p")
        || codec.to_lowercase().starts_with("m4b")
        || codec.to_lowercase().starts_with("mp4")
        || codec.to_lowercase().starts_with("3gp")
}

fn is_alac(codec: &str) -> bool {
    codec.to_lowercase().starts_with("alac")
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
            Self::AAC(_) => "aac",
            Self::ALAC(_) => "m4a",
            Self::FLAC(_) => "flac",
            Self::Opus(_) => "opus",
            Self::PCM(_) => "wav",
        }
    }

    #[must_use]
    pub const fn is_support_container_with_vcodec(&self, video_codec: &VideoCodec, container: &Container) -> bool {
        use AudioCodec::{Opus, AAC, ALAC, FLAC, PCM};
        use Container::{MKV, MOV, MP4, TS};
        use VideoCodec::{ProRes, AV1, H264, H265, VP9};

        matches!(
            (self, video_codec, container),
            (AAC(_), H264(_), _)
                | (AAC(_), H265(_), MP4 | MOV | MKV | TS)
                | (AAC(_) | ALAC(_) | FLAC(_) | Opus(_), AV1(_) | VP9(_), MP4 | MKV)
                | (AAC(_) | ALAC(_) | PCM(_), ProRes(_), MOV | MKV)
                | (ALAC(_), H264(_) | H265(_), MP4 | MOV | MKV)
                | (FLAC(_), H264(_) | H265(_), MP4 | MKV)
                | (FLAC(_) | Opus(_), ProRes(_), MKV)
                | (Opus(_), H264(_) | H265(_), MP4 | MKV | TS)
                | (PCM(_), H264(_) | H265(_), MOV | MKV)
                | (PCM(_), AV1(_) | VP9(_), MKV)
        )
    }

    #[must_use]
    pub const fn get_priority(&self) -> u8 {
        use AudioCodec::{Opus, AAC, ALAC, FLAC, PCM};

        match self {
            FLAC(_) => 1,
            ALAC(_) => 2,
            Opus(_) => 3,
            AAC(_) => 4,
            PCM(_) => 5,
        }
    }
}

impl<'a> TryFrom<&'a str> for AudioCodec<'a> {
    type Error = FormatError<'a>;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        match value {
            value if is_aac(value) => Ok(Self::AAC(value)),
            value if is_alac(value) => Ok(Self::ALAC(value)),
            value if is_flac(value) => Ok(Self::FLAC(value)),
            value if is_opus(value) => Ok(Self::Opus(value)),
            value if is_pcm(value) => Ok(Self::PCM(value)),
            _ => Err(FormatError::AudioCodecNotSupported { codec: value }),
        }
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct VideoFormat<'a> {
    pub id: &'a str,
    pub codec: VideoCodec<'a>,
    pub container: Container,
    pub height: Option<f64>,
    pub width: Option<f64>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> VideoFormat<'a> {
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
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct AudioFormat<'a> {
    pub id: &'a str,
    pub codec: AudioCodec<'a>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> AudioFormat<'a> {
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
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub enum FormatKind<'a> {
    Audio(AudioFormat<'a>),
    Video(VideoFormat<'a>),
    CombinedFormat(AudioFormat<'a>, VideoFormat<'a>),
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct AnyFormat {
    pub id: String,
    pub acodec: Option<String>,
    pub vcodec: Option<String>,
    pub container: Option<String>,
    pub height: Option<f64>,
    pub width: Option<f64>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'de> Deserialize<'de> for AnyFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct AnyFormatRaw {
            format_id: String,
            acodec: Option<String>,
            vcodec: Option<String>,
            container: Option<String>,
            height: Option<f64>,
            width: Option<f64>,
            filesize: Option<f64>,
            filesize_approx: Option<f64>,
        }

        let raw = AnyFormatRaw::deserialize(deserializer)?;

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

impl AnyFormat {
    #[allow(clippy::similar_names)]
    pub fn kind(&self) -> Result<FormatKind<'_>, FormatError<'_>> {
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

            let audio_format = AudioFormat::new(self.id.as_str(), acodec, self.filesize, self.filesize_approx);

            let video_format = VideoFormat::new(
                self.id.as_str(),
                vcodec,
                container,
                self.height,
                self.width,
                self.filesize,
                self.filesize_approx,
            );

            return Ok(FormatKind::CombinedFormat(audio_format, video_format));
        }

        if let Some(acodec) = acodec {
            let acodec = AudioCodec::try_from(acodec.as_str())?;
            let audio_format = AudioFormat::new(self.id.as_str(), acodec, self.filesize, self.filesize_approx);

            Ok(FormatKind::Audio(audio_format))
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

            let video_format = VideoFormat::new(
                self.id.as_str(),
                vcodec,
                container,
                self.height,
                self.width,
                self.filesize,
                self.filesize_approx,
            );

            Ok(FormatKind::Video(video_format))
        } else {
            Err(FormatError::AudioAndVideoCodecsEmpty)
        }
    }
}

impl<'a> TryFrom<&'a AnyFormat> for FormatKind<'a> {
    type Error = FormatError<'a>;

    fn try_from(value: &'a AnyFormat) -> Result<Self, Self::Error> {
        value.kind()
    }
}
