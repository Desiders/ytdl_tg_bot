use crate::{
    entities::{language::Language, Range, ShortMedia},
    handlers_utils::progress,
    interactors::{get_media, Interactor as _},
    services::messenger::{AnswerInlineQueryRequest, InlineQueryArticle, MessengerPort, TextFormat},
    services::yt_toolkit::GetVideoInfoErrorKind,
    utils::format_error_report,
};

use froodi::Inject;
use telers::{
    event::{telegram::HandlerResult, EventReturn},
    types::InlineQuery,
    Extension,
};
use tracing::{debug, error, instrument, warn};
use url::Url;
use uuid::Uuid;

const SELECT_INLINE_QUERY_CACHE_TIME: i64 = 86400; // 24 hours

#[instrument(skip_all, fields(query_id, url = url.as_str()))]
pub async fn select_by_url<Messenger>(
    InlineQuery { id: query_id, .. }: InlineQuery,
    Extension(url): Extension<Url>,
    Inject(messenger): Inject<Messenger>,
    Inject(get_basic_info_media): Inject<get_media::GetShortMediaByURL>,
    Inject(get_media): Inject<get_media::GetUncachedVideoByURL>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    debug!("Got url");

    let media_many: Vec<ShortMedia> = match get_basic_info_media.execute(get_media::GetShortMediaByURLInput { url: &url }).await {
        Ok(val) => val.into_iter().map(Into::into).collect(),
        Err(err) => {
            if let GetVideoInfoErrorKind::GetVideoId(err) = err {
                warn!(%err, "Unsupported YT Toolkit URL");
            } else {
                error!(err = format_error_report(&err), "Get YT Toolkit media error");
            }
            match get_media
                .execute(get_media::GetUncachedMediaByURLInput {
                    url: &url,
                    playlist_range: &Range::default(),
                    audio_language: &Language::default(),
                })
                .await
            {
                Ok(playlist) => playlist.inner.into_iter().map(|(val, _)| val.into()).collect(),
                Err(err) => {
                    error!(err = format_error_report(&err), "Get info error");
                    progress::is_error_in_inline_query(&*messenger, &query_id, "Sorry, an error to get media").await?;
                    return Ok(EventReturn::Finish);
                }
            }
        }
    };
    if media_many.is_empty() {
        warn!("Empty playlist");
        progress::is_error_in_inline_query(&*messenger, &query_id, "Playlist is empty").await?;
        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryArticle> = Vec::with_capacity(media_many.len() * 2);
    for media in media_many {
        let title = media.title.as_deref().unwrap_or("No name");
        let thumbnail = media.thumbnail.map(|val| val.to_string());
        let result_id = Uuid::new_v4();

        results.push(InlineQueryArticle {
            id: format!("video_{result_id}"),
            title: title.to_owned(),
            content_text: "🔍 Preparing download...".to_owned(),
            content_format: Some(TextFormat::Html),
            thumbnail_url: thumbnail.clone(),
            description: Some("Click to download video".to_owned()),
            callback_data: Some("video_download".to_owned()),
        });
        results.push(InlineQueryArticle {
            id: format!("audio_{result_id}"),
            title: "↑".to_owned(),
            content_text: "🔍 Preparing download...".to_owned(),
            content_format: Some(TextFormat::Html),
            thumbnail_url: thumbnail,
            description: Some("Click to download audio".to_owned()),
            callback_data: Some("audio_download".to_owned()),
        });
    }

    messenger
        .answer_inline_query(AnswerInlineQueryRequest {
            query_id: &query_id,
            results,
            cache_time: SELECT_INLINE_QUERY_CACHE_TIME,
            is_personal: false,
        })
        .await?;
    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, text))]
pub async fn select_by_text<Messenger>(
    InlineQuery {
        id: query_id, query: text, ..
    }: InlineQuery,
    Inject(messenger): Inject<Messenger>,
    Inject(get_basic_info_media): Inject<get_media::SearchMediaInfo>,
) -> HandlerResult
where
    Messenger: MessengerPort,
{
    debug!("Got text");

    let media_many: Vec<ShortMedia> = match get_basic_info_media
        .execute(get_media::SearchMediaInfoInput { text: text.as_ref() })
        .await
    {
        Ok(val) => val
            .into_iter()
            .map(Into::into)
            .enumerate()
            .filter(|(index, _)| *index < 25)
            .map(|(_, video)| video)
            .collect(),
        Err(err) => {
            error!(err = format_error_report(&err), "Search media error");
            progress::is_error_in_inline_query(&*messenger, &query_id, "Sorry, an error to search media").await?;
            return Ok(EventReturn::Finish);
        }
    };
    if media_many.is_empty() {
        warn!("Empty playlist");
        progress::is_error_in_inline_query(&*messenger, &query_id, "Playlist is empty").await?;
        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryArticle> = Vec::with_capacity(media_many.len() * 2);
    for media in media_many {
        let title = media.title.as_deref().unwrap_or("No name");
        let thumbnail = media.thumbnail.map(|val| val.to_string());
        let id = &media.id;

        results.push(InlineQueryArticle {
            id: format!("video_{id}"),
            title: title.to_owned(),
            content_text: "🔍 Preparing download...".to_owned(),
            content_format: Some(TextFormat::Html),
            thumbnail_url: thumbnail.clone(),
            description: Some("Click to download video".to_owned()),
            callback_data: Some("video_download".to_owned()),
        });
        results.push(InlineQueryArticle {
            id: format!("audio_{id}"),
            title: "↑".to_owned(),
            content_text: "🔍 Preparing download...".to_owned(),
            content_format: Some(TextFormat::Html),
            thumbnail_url: thumbnail,
            description: Some("Click to download audio".to_owned()),
            callback_data: Some("audio_download".to_owned()),
        });
    }

    messenger
        .answer_inline_query(AnswerInlineQueryRequest {
            query_id: &query_id,
            results,
            cache_time: SELECT_INLINE_QUERY_CACHE_TIME,
            is_personal: false,
        })
        .await?;
    Ok(EventReturn::Finish)
}
