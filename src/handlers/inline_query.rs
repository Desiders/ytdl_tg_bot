use crate::{
    entities::{Range, ShortInfo, UrlWithParams},
    handlers_utils::error,
    interactors::{
        GetMedaInfoByURLInput, GetMediaInfoByURL, GetShortMediaByURLInfo, GetShortMediaInfoByURLInput, Interactor, SearchMediaInfo,
        SearchMediaInfoInput,
    },
    services::yt_toolkit::GetVideoInfoErrorKind,
    utils::format_error_report,
};

use froodi::InjectTransient;
use telers::{
    enums::ParseMode,
    event::{telegram::HandlerResult, EventReturn},
    methods::AnswerInlineQuery,
    types::{
        InlineKeyboardButton, InlineKeyboardMarkup, InlineQuery, InlineQueryResult, InlineQueryResultArticle, InputTextMessageContent,
    },
    utils::text::{html_code, html_quote},
    Bot, Extension,
};
use tracing::{event, instrument, Level};
use url::Host;
use uuid::Uuid;

const SELECT_INLINE_QUERY_CACHE_TIME: i64 = 86400; // 24 hours

#[instrument(skip_all, fields(query_id, url = url.as_str()))]
pub async fn select_by_url(
    bot: Bot,
    InlineQuery { id: query_id, .. }: InlineQuery,
    Extension(UrlWithParams { url, .. }): Extension<UrlWithParams>,
    InjectTransient(mut get_short_media_info): InjectTransient<GetShortMediaByURLInfo>,
    InjectTransient(mut get_media_info): InjectTransient<GetMediaInfoByURL>,
) -> HandlerResult {
    event!(Level::DEBUG, "Got url");

    let videos: Vec<ShortInfo> = match get_short_media_info.execute(GetShortMediaInfoByURLInput::new(&url)).await {
        Ok(val) => val.into_iter().map(Into::into).collect(),
        Err(err) => {
            if let GetVideoInfoErrorKind::GetVideoId(err) = err {
                event!(Level::WARN, %err, "Unsupported YT Toolkit URL");
            } else {
                event!(Level::ERROR, err = format_error_report(&err), "Get YT Toolkit media info error");
            }

            match get_media_info.execute(GetMedaInfoByURLInput::new(&url, &Range::default())).await {
                Ok(val) => val.into_iter().map(Into::into).collect(),
                Err(err) => {
                    event!(Level::ERROR, err = format_error_report(&err), "Get info err");
                    error::occured_in_inline_query_occured(&bot, query_id.as_ref(), "Sorry, an error to get media info").await?;
                    return Ok(EventReturn::Finish);
                }
            }
        }
    };
    if videos.is_empty() {
        event!(Level::WARN, "Playlist empty");
        error::occured_in_inline_query_occured(&bot, &query_id, "Playlist empty").await?;
        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos.len());
    for video in videos {
        let title = video.title.as_deref().unwrap_or("No name");
        let title_html = html_code(html_quote(title));
        let thumbnail_urls = video.thumbnail_urls(url.host().as_ref());
        let result_id = Uuid::new_v4();

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{result_id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail_urls.first())
            .description("Click to download video")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading video...").callback_data("video_download")
            ]]))
            .into(),
        );
        results.push(
            InlineQueryResultArticle::new(
                format!("audio_{result_id}"),
                "↑",
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail_urls.first())
            .description("Click to download audio")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading audio...").callback_data("audio_download")
            ]]))
            .into(),
        );
    }

    bot.send(
        AnswerInlineQuery::new(query_id, results)
            .is_personal(false)
            .cache_time(SELECT_INLINE_QUERY_CACHE_TIME),
    )
    .await?;

    Ok(EventReturn::Finish)
}

#[instrument(skip_all, fields(query_id, text))]
pub async fn select_by_text(
    bot: Bot,
    InlineQuery {
        id: query_id, query: text, ..
    }: InlineQuery,
    InjectTransient(mut search_media_info): InjectTransient<SearchMediaInfo>,
) -> HandlerResult {
    event!(Level::DEBUG, "Got text");

    let videos: Vec<ShortInfo> = match search_media_info.execute(SearchMediaInfoInput::new(text.as_ref())).await {
        Ok(val) => val
            .into_iter()
            .map(Into::into)
            .enumerate()
            .filter(|(index, _)| *index < 25)
            .map(|(_, video)| video)
            .collect(),
        Err(err) => {
            event!(Level::ERROR, err = format_error_report(&err), "Search media info err");
            error::occured_in_inline_query_occured(&bot, query_id.as_ref(), "Sorry, an error to search media info").await?;
            return Ok(EventReturn::Finish);
        }
    };
    if videos.is_empty() {
        event!(Level::WARN, "Playlist empty");
        error::occured_in_inline_query_occured(&bot, &query_id, "Playlist empty").await?;
        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(videos.len());
    for video in videos {
        let title = video.title.as_deref().unwrap_or("No name");
        let title_html = html_code(html_quote(title));
        let thumbnail_urls = video.thumbnail_urls(Some(&Host::Domain("youtube.com")));
        let id = &video.id;

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail_urls.first())
            .description("Click to download video")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading video...").callback_data("video_download")
            ]]))
            .into(),
        );
        results.push(
            InlineQueryResultArticle::new(
                format!("audio_{id}"),
                "↑",
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail_urls.first())
            .description("Click to download audio")
            .reply_markup(InlineKeyboardMarkup::new([[
                InlineKeyboardButton::new("Downloading audio...").callback_data("audio_download")
            ]]))
            .into(),
        );
    }

    bot.send(
        AnswerInlineQuery::new(query_id, results)
            .is_personal(false)
            .cache_time(SELECT_INLINE_QUERY_CACHE_TIME),
    )
    .await?;

    Ok(EventReturn::Finish)
}
