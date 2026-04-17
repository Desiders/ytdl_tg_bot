use tonic::{
    metadata::{Ascii, MetadataValue},
    service::Interceptor,
    Request, Status,
};
use tracing::warn;

#[derive(Clone)]
pub struct AuthInterceptor {
    tokens: Vec<Box<str>>,
}

impl AuthInterceptor {
    #[inline]
    #[must_use]
    pub const fn new(tokens: Vec<Box<str>>) -> Self {
        Self { tokens }
    }
}

impl Interceptor for AuthInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let remote_addr = request
            .remote_addr()
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| String::from("unknown"));
        let user_agent = request
            .metadata()
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown");

        let Some(header) = request.metadata().get("authorization") else {
            warn!(remote_addr, user_agent, "Rejected request without authorization header");
            return Err(Status::unauthenticated("Invalid token"));
        };

        if self.tokens.iter().any(|token| is_valid_token(header, token)) {
            Ok(request)
        } else {
            warn!(remote_addr, user_agent, "Rejected request with invalid authorization token");
            Err(Status::unauthenticated("Invalid token"))
        }
    }
}

fn is_valid_token(header: &MetadataValue<Ascii>, token: &str) -> bool {
    let Ok(header) = header.to_str() else {
        return false;
    };
    let Some(actual_token) = header.strip_prefix("Bearer ") else {
        return false;
    };
    actual_token == token
}
