use std::sync::Arc;

use proto::downloader::{
    node_cookie_manager_server::NodeCookieManager, Empty, ListNodeCookiesResponse, PushCookieRequest, RemoveCookieRequest,
};
use tonic::{Request, Response, Status};
use tracing::{error, info};

use crate::entities::Cookies;

pub struct CookieManagerService {
    pub cookies: Arc<Cookies>,
}

#[tonic::async_trait]
impl NodeCookieManager for CookieManagerService {
    async fn push_cookie(&self, request: Request<PushCookieRequest>) -> Result<Response<Empty>, Status> {
        let request = request.into_inner();

        self.cookies
            .upsert_cookie(&request.cookie_id, &request.domain, &request.data)
            .map_err(|err| {
                error!(cookie_id = %request.cookie_id, domain = %request.domain, %err, "Failed to upsert cookie");
                Status::internal(format!("Cookie upsert failed: {err}"))
            })?;

        info!(cookie_id = %request.cookie_id, domain = %request.domain, "Cookie upserted");
        Ok(Response::new(Empty {}))
    }

    async fn remove_cookie(&self, request: Request<RemoveCookieRequest>) -> Result<Response<Empty>, Status> {
        let request = request.into_inner();

        self.cookies.remove_cookie(&request.cookie_id).map_err(|err| {
            error!(cookie_id = %request.cookie_id, %err, "Failed to remove cookie");
            Status::internal(format!("Cookie remove failed: {err}"))
        })?;

        info!(cookie_id = %request.cookie_id, "Cookie removed");
        Ok(Response::new(Empty {}))
    }

    async fn list_node_cookies(&self, _request: Request<Empty>) -> Result<Response<ListNodeCookiesResponse>, Status> {
        let cookie_ids = self.cookies.get_cookie_ids();
        Ok(Response::new(ListNodeCookiesResponse { cookie_ids }))
    }
}
