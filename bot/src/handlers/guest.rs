use crate::{
    interactors::{enqueue_download, Interactor as _},
    locale::Locale,
    services::messenger::MessengerPort,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::UpdateGuestMessage,
    Extension,
};
use tracing::instrument;
use url::Url;

#[instrument(skip_all)]
pub async fn download<Messenger>(
    update: UpdateGuestMessage,
    Extension(url): Extension<Url>,
    Inject(interactor): Inject<enqueue_download::EnqueueGuestDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    enqueue(&update, Some(&url), &interactor).await
}

#[instrument(skip_all)]
pub async fn help<Messenger>(
    update: UpdateGuestMessage,
    Inject(interactor): Inject<enqueue_download::EnqueueGuestDownload<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    enqueue(&update, None, &interactor).await
}

async fn enqueue<Messenger>(
    update: &UpdateGuestMessage,
    url: Option<&Url>,
    interactor: &enqueue_download::EnqueueGuestDownload<Messenger>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let message = &update.guest_message;
    let Some(query_id) = message.guest_query_id().filter(|id| !id.is_empty()) else {
        return Ok(EventReturn::Finish);
    };
    let locale = Locale::from_code(message.from().and_then(|user| user.language_code.as_deref()));
    interactor
        .execute(enqueue_download::EnqueueGuestInput { query_id, url, locale })
        .await?;
    Ok(EventReturn::Finish)
}
