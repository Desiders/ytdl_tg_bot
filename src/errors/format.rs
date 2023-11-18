#[allow(clippy::module_name_repetitions)]
#[derive(thiserror::Error, Debug)]
#[error("Format error: {0}")]
pub enum FormatError<'a> {
    #[error("Audio codec `{codec}` is not supported")]
    AudioCodecNotSupported { codec: &'a str },
    #[error("Video codec `{codec}` is not supported")]
    VideoCodecNotSupported { codec: &'a str },
    #[error("Container `{container}` is not supported")]
    ContainerNotSupported { container: &'a str },
    #[error("Container `{container}` is not supported by video codec `{codec}`")]
    ContainerNotSupportedByVideoCodec { container: Box<str>, codec: Box<str> },
    #[error("Audio and video codecs are empty")]
    AudioAndVideoCodecsEmpty,
    #[error("Video container is empty")]
    VideoContainerEmpty,
}
