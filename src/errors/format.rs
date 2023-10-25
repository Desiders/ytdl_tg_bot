use crate::models::Format;

#[allow(clippy::module_name_repetitions)]
#[derive(thiserror::Error, Debug)]
#[error("Error while extracting format `{format:?}`: {message}")]
pub enum FormatError<'a> {
    #[error("Format `{format:?}` does not have a format id")]
    FormatIdNotFound { format: &'a Format },
}
