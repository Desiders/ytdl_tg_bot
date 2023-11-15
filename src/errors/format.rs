#[allow(clippy::module_name_repetitions)]
#[derive(thiserror::Error, Debug)]
#[error("Format error: {0}")]
pub enum FormatError<'a> {
    #[error("Format id `{id}` is not supported")]
    FormatIdNotSupported { id: &'a str },
}
