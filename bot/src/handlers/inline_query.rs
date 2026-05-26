use crate::{
    interactors::{inline_query, Interactor as _},
    locale::Locale,
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
    InlineQuery { id: query_id, from, .. }: InlineQuery,
    Extension(url): Extension<Url>,
    Inject(interactor): Inject<inline_query::SelectByUrl<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let locale = Locale::from_code(from.language_code.as_deref());
    interactor
        .execute(inline_query::SelectByUrlInput {
            query_id: &query_id,
            url: &url,
            locale: locale.as_str(),
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, text))]
pub async fn select_by_text<Messenger>(
    InlineQuery {
        id: query_id,
        query: text,
        from,
        ..
    }: InlineQuery,
    Inject(interactor): Inject<inline_query::SelectByText<Messenger>>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    let locale = Locale::from_code(from.language_code.as_deref());
    interactor
        .execute(inline_query::SelectByTextInput {
            query_id: &query_id,
            text: text.as_ref(),
            locale: locale.as_str(),
        })
        .await?;
    Ok(EventReturn::Finish)
}
