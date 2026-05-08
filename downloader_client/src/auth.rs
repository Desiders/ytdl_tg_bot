use tonic::{
    metadata::{errors::InvalidMetadataValue, Ascii, MetadataValue},
    Request,
};

/// Builds a tonic request with the downloader bearer token in metadata.
///
/// # Errors
///
/// Returns [`InvalidMetadataValue`] if the token cannot be encoded as an ASCII
/// metadata value.
pub fn authenticated_request<T>(message: T, token: &str) -> Result<Request<T>, InvalidMetadataValue> {
    let mut request = Request::new(message);
    let value: MetadataValue<Ascii> = format!("Bearer {token}").parse()?;
    request.metadata_mut().insert("authorization", value);
    Ok(request)
}
