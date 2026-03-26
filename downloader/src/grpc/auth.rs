use std::sync::Arc;
use tonic::{
    metadata::{Ascii, MetadataValue},
    service::Interceptor,
    Request, Status,
};
use tracing::warn;

#[derive(Clone)]
pub struct AuthInterceptor {
    tokens: Arc<[Box<str>]>,
}

impl AuthInterceptor {
    #[must_use]
    pub fn new(tokens: Vec<Box<str>>) -> Self {
        Self {
            tokens: Arc::from(tokens.into_boxed_slice()),
        }
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let Some(header) = request.metadata().get("authorization") else {
            warn!("Rejected request without authorization header");
            return Err(Status::unauthenticated("Invalid token"));
        };

        if is_valid_token(header, &self.tokens) {
            Ok(request)
        } else {
            warn!("Rejected request with invalid authorization token");
            Err(Status::unauthenticated("Invalid token"))
        }
    }
}

fn is_valid_token(header: &MetadataValue<Ascii>, tokens: &[Box<str>]) -> bool {
    let Ok(header) = header.to_str() else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return false;
    };
    tokens.iter().any(|candidate| candidate.as_ref() == token)
}
