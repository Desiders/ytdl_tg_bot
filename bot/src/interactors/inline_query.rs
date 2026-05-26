use std::sync::Arc;

use rust_i18n::t;
use telers::errors::HandlerError;
use tracing::{debug, error, instrument, warn};
use url::Url;
use uuid::Uuid;

use crate::{
    entities::ShortMedia,
    handlers_utils::progress,
    interactors::Interactor,
    services::{
        get_media,
        messenger::{AnswerInlineQueryRequest, InlineQueryArticle, MessengerPort, TextFormat},
        yt_toolkit::GetVideoInfoErrorKind,
    },
    utils::ErrorFormatter,
};

const SELECT_INLINE_QUERY_CACHE_TIME: i64 = 86_400;

pub struct SelectByUrl<Messenger> {
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
    pub get_basic_info_media: Arc<get_media::GetShortMediaByURL>,
}

pub struct SelectByUrlInput<'a> {
    pub query_id: &'a str,
    pub url: &'a Url,
    pub locale: &'a str,
}

impl<Messenger> Interactor<SelectByUrlInput<'_>> for &SelectByUrl<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(query_id = input.query_id, url = input.url.as_str()))]
    async fn execute(self, input: SelectByUrlInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Got url");

        let media_many: Vec<ShortMedia> = match self
            .get_basic_info_media
            .execute(get_media::GetShortMediaByURLInput { url: input.url })
            .await
        {
            Ok(val) => val.into_iter().map(Into::into).collect(),
            Err(err) => {
                if let GetVideoInfoErrorKind::GetVideoId(err) = err {
                    warn!(%err, "Unsupported YT Toolkit URL");
                } else {
                    error!(err = %self.error_formatter.format(&err), "Get YT Toolkit media error");
                }

                // No fast preview source — offer a download entry without a thumbnail or media
                // name (the loop renders a missing title as "No name") rather than falling back
                // to yt-dlp, which is far too slow for inline. The download itself uses the URL
                // forwarded via the chosen-inline `Extension`.
                vec![ShortMedia {
                    id: String::new(),
                    title: None,
                    thumbnail: None,
                }]
            }
        };

        if media_many.is_empty() {
            warn!("Empty playlist");
            if let Err(err) = progress::is_error_in_inline_query(
                self.messenger.as_ref(),
                input.query_id,
                t!("download.playlist_empty", locale = input.locale).as_ref(),
            )
            .await
            {
                error!(err = %self.error_formatter.format(&err), "Answer inline query error");
            }
            return Ok(());
        }

        let mut results = Vec::with_capacity(media_many.len() * 2);
        let no_name = t!("inline.no_name", locale = input.locale);
        for media in media_many {
            let title = media.title.as_deref().unwrap_or(no_name.as_ref());
            let thumbnail = media.thumbnail.map(|val| val.to_string());
            let result_id = Uuid::new_v4();

            results.push(InlineQueryArticle {
                id: format!("video_{result_id}"),
                title: title.to_owned(),
                content_text: t!("download.preparing", locale = input.locale).into_owned(),
                content_format: Some(TextFormat::Html),
                thumbnail_url: thumbnail.clone(),
                description: Some(t!("inline.download_video", locale = input.locale).into_owned()),
                callback_data: Some("video_download".to_owned()),
            });
            results.push(InlineQueryArticle {
                id: format!("audio_{result_id}"),
                title: "↑".to_owned(),
                content_text: t!("download.preparing", locale = input.locale).into_owned(),
                content_format: Some(TextFormat::Html),
                thumbnail_url: thumbnail,
                description: Some(t!("inline.download_audio", locale = input.locale).into_owned()),
                callback_data: Some("audio_download".to_owned()),
            });
        }

        if let Err(err) = self
            .messenger
            .answer_inline_query(AnswerInlineQueryRequest {
                query_id: input.query_id,
                results,
                cache_time: SELECT_INLINE_QUERY_CACHE_TIME,
                is_personal: false,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Answer inline query error");
        }

        Ok(())
    }
}

pub struct SelectByText<Messenger> {
    pub error_formatter: Arc<ErrorFormatter>,
    pub messenger: Arc<Messenger>,
    pub get_basic_info_media: Arc<get_media::SearchMediaInfo>,
}

pub struct SelectByTextInput<'a> {
    pub query_id: &'a str,
    pub text: &'a str,
    pub locale: &'a str,
}

impl<Messenger> Interactor<SelectByTextInput<'_>> for &SelectByText<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all, fields(query_id = input.query_id, text = input.text))]
    async fn execute(self, input: SelectByTextInput<'_>) -> Result<Self::Output, Self::Err> {
        debug!("Got text");

        let media_many: Vec<ShortMedia> = match self
            .get_basic_info_media
            .execute(get_media::SearchMediaInfoInput { text: input.text })
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
                error!(err = %self.error_formatter.format(&err), "Search media error");
                if let Err(err) = progress::is_error_in_inline_query(
                    self.messenger.as_ref(),
                    input.query_id,
                    t!("download.error_search_media", locale = input.locale).as_ref(),
                )
                .await
                {
                    error!(err = %self.error_formatter.format(&err), "Answer inline query error");
                }
                return Ok(());
            }
        };

        if media_many.is_empty() {
            warn!("Empty playlist");
            if let Err(err) = progress::is_error_in_inline_query(
                self.messenger.as_ref(),
                input.query_id,
                t!("download.playlist_empty", locale = input.locale).as_ref(),
            )
            .await
            {
                error!(err = %self.error_formatter.format(&err), "Answer inline query error");
            }
            return Ok(());
        }

        let mut results = Vec::with_capacity(media_many.len() * 2);
        let no_name = t!("inline.no_name", locale = input.locale);
        for media in media_many {
            let title = media.title.as_deref().unwrap_or(no_name.as_ref());
            let thumbnail = media.thumbnail.map(|val| val.to_string());
            let id = &media.id;

            results.push(InlineQueryArticle {
                id: format!("video_{id}"),
                title: title.to_owned(),
                content_text: t!("download.preparing", locale = input.locale).into_owned(),
                content_format: Some(TextFormat::Html),
                thumbnail_url: thumbnail.clone(),
                description: Some(t!("inline.download_video", locale = input.locale).into_owned()),
                callback_data: Some("video_download".to_owned()),
            });
            results.push(InlineQueryArticle {
                id: format!("audio_{id}"),
                title: "↑".to_owned(),
                content_text: t!("download.preparing", locale = input.locale).into_owned(),
                content_format: Some(TextFormat::Html),
                thumbnail_url: thumbnail,
                description: Some(t!("inline.download_audio", locale = input.locale).into_owned()),
                callback_data: Some("audio_download".to_owned()),
            });
        }

        if let Err(err) = self
            .messenger
            .answer_inline_query(AnswerInlineQueryRequest {
                query_id: input.query_id,
                results,
                cache_time: SELECT_INLINE_QUERY_CACHE_TIME,
                is_personal: false,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Answer inline query error");
        }

        Ok(())
    }
}
