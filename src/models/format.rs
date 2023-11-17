use crate::errors::FormatError;

use lazy_static::lazy_static;
use serde::Deserialize;
use std::collections::HashMap;

const AUDIO_CODECS: [&str; 5] = ["AAC", "ALAC", "FLAC", "Opus", "PCM"];
const VIDEO_CODECS: [&str; 4] = ["H.264", "HEVC", "AV1", "ProRes"];

lazy_static! {
    static ref AUDIO_AND_VIDEO_CODECS_AND_ITS_CONTAINERS: HashMap<&'static str, HashMap<&'static str, Box<[&'static str]>>> = {
        let mut map: HashMap<&'static str, HashMap<&'static str, Box<[&'static str]>>> = HashMap::new();

        map.insert("AAC", {
            let mut map: HashMap<&'static str, Box<[&'static str]>> = HashMap::new();

            map.insert("H.264", Box::new(["Any"]));
            map.insert("HEVC", Box::new(["MP4", "MOV", "MKV", "TC"]));
            map.insert("AV1", Box::new(["MP4", "MKV"]));
            map.insert("ProRes", Box::new(["MOV", "MKV"]));
            map
        });

        map.insert("ALAC", {
            let mut map: HashMap<&'static str, Box<[&'static str]>> = HashMap::new();

            map.insert("H.264", Box::new(["MP4", "MOV", "MKV"]));
            map.insert("HEVC", Box::new(["MP4", "MOV", "MKV"]));
            map.insert("AV1", Box::new(["MP4", "MKV"]));
            map.insert("ProRes", Box::new(["MOV", "MKV"]));
            map
        });

        map.insert("FLAC", {
            let mut map: HashMap<&'static str, Box<[&'static str]>> = HashMap::new();

            map.insert("H.264", Box::new(["MP4", "MKV"]));
            map.insert("HEVC", Box::new(["MP4", "MKV"]));
            map.insert("AV1", Box::new(["MP4", "MKV"]));
            map.insert("ProRes", Box::new(["MKV"]));
            map
        });

        map.insert("Opus", {
            let mut map: HashMap<&'static str, Box<[&'static str]>> = HashMap::new();

            map.insert("H.264", Box::new(["MP4", "MKV", "TS"]));
            map.insert("HEVC", Box::new(["MP4", "MKV", "TS"]));
            map.insert("AV1", Box::new(["MP4", "MKV"]));
            map.insert("ProRes", Box::new(["MKV"]));
            map
        });

        map.insert("PCM", {
            let mut map: HashMap<&'static str, Box<[&'static str]>> = HashMap::new();

            map.insert("H.264", Box::new(["MOV", "MKV"]));
            map.insert("HEVC", Box::new(["MOV", "MKV"]));
            map.insert("AV1", Box::new(["MKV"]));
            map.insert("ProRes", Box::new(["MOV", "MKV"]));
            map
        });

        map
    };
    static ref AUDIO_IDS_AND_CODEC: HashMap<&'static str, &'static str> = {
        let map = HashMap::from([
            ("139", "AAC"),
            ("140", "AAC"),
            ("141", "AAC"),
            ("249", "Opus"),
            ("250", "Opus"),
            ("251", "Opus"),
            ("256", "Opus"),
            ("258", "Opus"),
            ("327", "AAC"),
            ("338", "Opus"),
            ("599", "AAC"),
            ("600", "Opus"),
        ]);

        for codec in map.values() {
            assert!(
                AUDIO_CODECS.contains(codec),
                "Audio codec `{codec}` does not exist in `AUDIO_CODECS`"
            );
        }

        map
    };
    static ref AUDIO_IDS_AND_PRIORITY: HashMap<&'static str, u8> = HashMap::from([
        ("139", 1),
        ("140", 4),
        ("141", 7),
        ("249", 2),
        ("250", 3),
        ("251", 5),
        ("256", 6),
        ("258", 8),
        ("327", 7),
        ("338", 9),
        ("599", 1),
        ("600", 1),
    ]);
    static ref VIDEO_IDS_AND_CODEC_CONTAINER_PAIR: HashMap<&'static str, (&'static str, &'static str)> = {
        let map = HashMap::from([
            ("133", ("H.264", "MP4")),
            ("134", ("H.264", "MP4")),
            ("135", ("H.264", "MP4")),
            ("136", ("H.264", "MP4")),
            ("137", ("H.264", "MP4")),
            ("138", ("H.264", "MP4")),
            ("160", ("H.264", "MP4")),
            ("216", ("H.264", "MP4")),
            ("298", ("H.264", "MP4")),
            ("299", ("H.264", "MP4")),
            ("394", ("AV1", "MP4")),
            ("395", ("AV1", "MP4")),
            ("396", ("AV1", "MP4")),
            ("397", ("AV1", "MP4")),
            ("398", ("AV1", "MP4")),
            ("399", ("AV1", "MP4")),
            ("400", ("AV1", "MP4")),
            ("401", ("AV1", "MP4")),
            ("402", ("AV1", "MP4")),
            ("571", ("AV1", "MP4")),
            ("694", ("AV1", "MP4")),
            ("695", ("AV1", "MP4")),
            ("696", ("AV1", "MP4")),
            ("697", ("AV1", "MP4")),
            ("698", ("AV1", "MP4")),
            ("699", ("AV1", "MP4")),
            ("700", ("AV1", "MP4")),
            ("701", ("AV1", "MP4")),
            ("702", ("AV1", "MP4")),
        ]);

        for (codec, _) in map.values() {
            assert!(
                VIDEO_CODECS.contains(codec),
                "Video codec `{codec}` does not exist in `VIDEO_CODECS`"
            );
        }

        map
    };
    static ref VIDEO_IDS_AND_PRIORITY: HashMap<&'static str, u8> = HashMap::from([
        ("133", 2),
        ("134", 3),
        ("135", 4),
        ("136", 5),
        ("137", 6),
        ("138", 9),
        ("160", 1),
        ("216", 6),
        ("298", 5),
        ("299", 6),
        ("394", 1),
        ("395", 2),
        ("396", 3),
        ("397", 4),
        ("398", 5),
        ("399", 6),
        ("400", 7),
        ("401", 8),
        ("402", 9),
        ("571", 9),
        ("694", 1),
        ("695", 2),
        ("696", 3),
        ("697", 4),
        ("698", 5),
        ("699", 6),
        ("700", 7),
        ("701", 8),
        ("702", 9),
    ]);
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct VideoFormat<'a> {
    pub id: &'a str,
    pub priority: u8,
    pub codec: &'static str,
    pub container: &'static str,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> VideoFormat<'a> {
    pub fn new(id: &'a str, codec: &'static str, container: &'static str, filesize: Option<f64>, filesize_approx: Option<f64>) -> Self {
        Self {
            id,
            priority: *VIDEO_IDS_AND_PRIORITY.get(id).unwrap(),
            codec,
            container,
            filesize,
            filesize_approx,
        }
    }

    #[must_use]
    pub fn get_extension(&self) -> &'static str {
        match self.container {
            "MP4" => "mp4",
            _ => unreachable!(),
        }
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct AudioFormat<'a> {
    pub id: &'a str,
    pub priority: u8,
    pub codec: &'static str,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl<'a> AudioFormat<'a> {
    pub fn new(id: &'a str, codec: &'static str, filesize: Option<f64>, filesize_approx: Option<f64>) -> Self {
        Self {
            id,
            priority: *AUDIO_IDS_AND_PRIORITY.get(id).unwrap(),
            codec,
            filesize,
            filesize_approx,
        }
    }

    #[must_use]
    pub fn support_video_format(&self, video_format: &VideoFormat) -> bool {
        let container = AUDIO_AND_VIDEO_CODECS_AND_ITS_CONTAINERS
            .get(self.codec)
            .and_then(|map| map.get(video_format.codec))
            .and_then(|containers| containers.iter().find(|container| container == &&video_format.container));

        container.is_some()
    }
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub enum FormatKind<'a> {
    Audio(AudioFormat<'a>),
    Video(VideoFormat<'a>),
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone, Deserialize)]
pub struct AnyFormat {
    #[serde(rename = "format_id")]
    pub id: String,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl AnyFormat {
    pub fn kind(&self) -> Result<FormatKind<'_>, FormatError<'_>> {
        if let Some(codec) = AUDIO_IDS_AND_CODEC.get(self.id.as_str()) {
            Ok(FormatKind::Audio(AudioFormat::new(
                self.id.as_str(),
                codec,
                self.filesize,
                self.filesize_approx,
            )))
        } else if let Some((codec, container)) = VIDEO_IDS_AND_CODEC_CONTAINER_PAIR.get(self.id.as_str()) {
            Ok(FormatKind::Video(VideoFormat::new(
                self.id.as_str(),
                codec,
                container,
                self.filesize,
                self.filesize_approx,
            )))
        } else {
            Err(FormatError::FormatIdNotSupported { id: self.id.as_str() })
        }
    }
}

impl<'a> TryFrom<&'a AnyFormat> for FormatKind<'a> {
    type Error = FormatError<'a>;

    fn try_from(value: &'a AnyFormat) -> Result<Self, Self::Error> {
        value.kind()
    }
}
