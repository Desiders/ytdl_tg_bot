use crate::{
    interactors::{inline_query, Interactor as _},
    services::messenger::MessengerPort,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::InlineQuery,
    Extension,
};
use tracing::instrument;
use url::Url;

#[instrument(skip_all, fields(query_id, url = url.as_str()))]
pub async fn select_by_url<Messenger>(
    InlineQuery { id: query_id, .. }: InlineQuery,
    Extension(url): Extension<Url>,
    Inject(interactor): Inject<inline_query::SelectByUrl<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(inline_query::SelectByUrlInput {
            query_id: &query_id,
            url: &url,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, text))]
pub async fn select_by_text<Messenger>(
    InlineQuery {
        id: query_id, query: text, ..
    }: InlineQuery,
    Inject(interactor): Inject<inline_query::SelectByText<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    interactor
        .execute(inline_query::SelectByTextInput {
            query_id: &query_id,
            text: text.as_ref(),
        })
        .await?;
    Ok(EventReturn::Finish)
}
