use tonic::{
    metadata::{errors::InvalidMetadataValue, Ascii, MetadataValue},
    Request,
};

pub fn authenticated_request<T>(message: T, token: &str) -> Result<Request<T>, InvalidMetadataValue> {
    let mut request = Request::new(message);
    let value: MetadataValue<Ascii> = format!("Bearer {token}").parse()?;
    request.metadata_mut().insert("authorization", value);
    Ok(request)
}
