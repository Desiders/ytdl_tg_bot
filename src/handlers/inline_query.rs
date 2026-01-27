use crate::{
    entities::{language::Language, Range, ShortMedia},
    handlers_utils::progress,
    interactors::{get_media, Interactor as _},
    services::yt_toolkit::GetVideoInfoErrorKind,
    utils::format_error_report,
};

use froodi::Inject;
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
use tracing::{debug, error, instrument, warn};
use url::Url;
use uuid::Uuid;

const SELECT_INLINE_QUERY_CACHE_TIME: i64 = 86400; // 24 hours

#[instrument(skip_all, fields(query_id, url = url.as_str()))]
pub async fn select_by_url(
    bot: Bot,
    InlineQuery { id: query_id, .. }: InlineQuery,
    Extension(url): Extension<Url>,
    Inject(get_basic_info_media): Inject<get_media::GetShortMediaByURL>,
    Inject(get_media): Inject<get_media::GetUncachedVideoByURL>,
) -> HandlerResult {
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
                    error!(err = format_error_report(&err), "Get info err");
                    progress::is_error_in_inline_query(&bot, &query_id, "Sorry, an error to get media").await?;
                    return Ok(EventReturn::Finish);
                }
            }
        }
    };
    if media_many.is_empty() {
        warn!("Empty playlist");
        progress::is_error_in_inline_query(&bot, &query_id, "Playlist is empty").await?;
        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(media_many.len());
    for media in media_many {
        let title = media.title.as_deref().unwrap_or("No name");
        let title_html = html_code(html_quote(title));
        let thumbnail = media.thumbnail;
        let result_id = Uuid::new_v4();

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{result_id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail.clone())
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
            .thumbnail_url_option(thumbnail)
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
    Inject(get_basic_info_media): Inject<get_media::SearchMediaInfo>,
) -> HandlerResult {
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
            error!(err = format_error_report(&err), "Search media err");
            progress::is_error_in_inline_query(&bot, &query_id, "Sorry, an error to search media").await?;
            return Ok(EventReturn::Finish);
        }
    };
    if media_many.is_empty() {
        warn!("Empty playlist");
        progress::is_error_in_inline_query(&bot, &query_id, "Playlist is empty").await?;
        return Ok(EventReturn::Finish);
    }

    let mut results: Vec<InlineQueryResult> = Vec::with_capacity(media_many.len());
    for media in media_many {
        let title = media.title.as_deref().unwrap_or("No name");
        let title_html = html_code(html_quote(title));
        let thumbnail = media.thumbnail;
        let id = &media.id;

        results.push(
            InlineQueryResultArticle::new(
                format!("video_{id}"),
                title,
                InputTextMessageContent::new(&title_html).parse_mode(ParseMode::HTML),
            )
            .thumbnail_url_option(thumbnail.clone())
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
            .thumbnail_url_option(thumbnail)
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
