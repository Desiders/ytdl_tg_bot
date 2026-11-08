use std::{fmt::Write as _, str::FromStr as _, sync::Arc, time::Duration};

use proto::downloader::{MediaInfoRequest, MediaInfoResponse};
use rust_i18n::t;
use telers::{callback_data::CallbackData as _, errors::HandlerError, utils::text::html_quote};
use tokio::time::timeout;
use tracing::{error, instrument};
use url::Url;
use uuid::Uuid;

use crate::{
    entities::{language::Language, ChatConfig, DownloadJob, JobTarget, Params, Range},
    handlers_utils::progress,
    interactors::Interactor,
    locale::Locale,
    services::{
        messenger::{AnswerCallbackRequest, Button, ButtonAction, EditMenuRequest, Keyboard, MenuMessage, MessengerPort, TextFormat},
        node_router::{get_media_info, NodeRouter},
        quality_selection_cache::{QualityOption, QualitySelection, QualitySelectionCache},
        queue::RedisJobQueue,
    },
    utils::ErrorFormatter,
    value_objects::{
        quality::{QualityAction, SelectQuality},
        MediaType, VideoQuality,
    },
};

const PAGE_SIZE: usize = 8;

pub struct Open<Messenger> {
    messenger: Arc<Messenger>,
    node_router: Arc<NodeRouter>,
    cache: Arc<QualitySelectionCache>,
    error_formatter: Arc<ErrorFormatter>,
}

impl<Messenger> Open<Messenger> {
    pub const fn new(
        messenger: Arc<Messenger>,
        node_router: Arc<NodeRouter>,
        cache: Arc<QualitySelectionCache>,
        error_formatter: Arc<ErrorFormatter>,
    ) -> Self {
        Self {
            messenger,
            node_router,
            cache,
            error_formatter,
        }
    }
}

pub struct OpenInput<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    pub is_private: bool,
    pub url: Option<&'a Url>,
    pub params: &'a Params,
    pub chat_cfg: &'a ChatConfig,
}

impl<Messenger: MessengerPort> Interactor<OpenInput<'_>> for &Open<Messenger> {
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all)]
    async fn execute(self, input: OpenInput<'_>) -> Result<(), HandlerError> {
        let locale = input.chat_cfg.locale();
        if !input.is_private || input.params.0.contains_key("items") || input.url.is_none() {
            let key = if !input.is_private {
                "quality.private_only"
            } else if input.url.is_none() {
                "quality.usage"
            } else {
                "quality.single_video"
            };
            progress::new(
                self.messenger.as_ref(),
                &t!(key, locale = locale.as_str()),
                input.chat_id,
                Some(input.message_id),
                None,
            )
            .await
            .map_err(HandlerError::new)?;
            return Ok(());
        }
        let url = input.url.unwrap();
        let message = progress::new(
            self.messenger.as_ref(),
            &t!("download.preparing", locale = locale.as_str()),
            input.chat_id,
            Some(input.message_id),
            None,
        )
        .await
        .map_err(HandlerError::new)?;
        let target = MenuMessage::ChatMessage {
            chat_id: input.chat_id,
            message_id: message.message_id,
        };
        let language = input
            .params
            .0
            .get("lang")
            .map_or_else(Language::default, |lang| Language::from_str(lang).unwrap());
        let response = timeout(
            Duration::from_secs(180),
            get_media_info(
                &self.node_router,
                url.domain(),
                MediaInfoRequest {
                    url: url.to_string(),
                    audio_language: language.language.unwrap_or_default(),
                    playlist_range: Some(Range::default().into()),
                    media_type: "video".to_owned(),
                    max_file_size: self.node_router.max_file_size(),
                    list_video_formats: true,
                    ..Default::default()
                },
            ),
        )
        .await;
        let response = response
            .map_err(|err| err.to_string())
            .and_then(|response| response.map_err(|err| self.error_formatter.format(&err).into_owned()));
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                error!(%err, "Get quality options error");
                self.messenger
                    .edit_menu(EditMenuRequest {
                        target,
                        text: &progress::with_link(&t!("quality.error", locale = locale.as_str()), url, Some(TextFormat::Html)),
                        keyboard: &Keyboard { rows: vec![] },
                    })
                    .await
                    .map_err(HandlerError::new)?;
                return Ok(());
            }
        };
        let Some((title, options)) = quality_options(response) else {
            self.messenger
                .edit_menu(EditMenuRequest {
                    target,
                    text: &progress::with_link(&t!("quality.unavailable", locale = locale.as_str()), url, Some(TextFormat::Html)),
                    keyboard: &Keyboard { rows: vec![] },
                })
                .await
                .map_err(HandlerError::new)?;
            return Ok(());
        };
        let selection = QualitySelection {
            job: DownloadJob::new(
                MediaType::Video,
                Some(url.clone()),
                input.params.clone(),
                input.chat_cfg.clone(),
                input.chat_cfg.link_is_visible,
                JobTarget::Command {
                    chat_id: input.chat_id,
                    message_id: input.message_id,
                },
            ),
            title,
            options,
        };
        let session = Uuid::now_v7().simple().to_string();
        if let Err(err) = self.cache.set(input.chat_id, &session, &selection).await {
            error!(%err, "Save quality selection error");
            self.messenger
                .edit_menu(EditMenuRequest {
                    target,
                    text: &progress::with_link(&t!("quality.error", locale = locale.as_str()), url, Some(TextFormat::Html)),
                    keyboard: &Keyboard { rows: vec![] },
                })
                .await
                .map_err(HandlerError::new)?;
            return Ok(());
        }
        let (text, keyboard) = render(&selection, &session, 0, locale);
        self.messenger
            .edit_menu(EditMenuRequest {
                target,
                text: &text,
                keyboard: &keyboard,
            })
            .await
            .map_err(HandlerError::new)
    }
}

pub struct Select<Messenger> {
    messenger: Arc<Messenger>,
    cache: Arc<QualitySelectionCache>,
    queue: Arc<RedisJobQueue>,
}

impl<Messenger> Select<Messenger> {
    pub const fn new(messenger: Arc<Messenger>, cache: Arc<QualitySelectionCache>, queue: Arc<RedisJobQueue>) -> Self {
        Self { messenger, cache, queue }
    }
}

pub struct SelectInput<'a> {
    pub chat_id: i64,
    pub user_id: i64,
    pub message_id: i64,
    pub callback_id: &'a str,
    pub data: &'a SelectQuality,
    pub locale: Locale,
}

impl<Messenger: MessengerPort> Interactor<SelectInput<'_>> for &Select<Messenger> {
    type Output = ();
    type Err = HandlerError;

    #[instrument(skip_all)]
    async fn execute(self, input: SelectInput<'_>) -> Result<(), HandlerError> {
        if input.chat_id != input.user_id {
            return Ok(());
        }
        let selection = match self.cache.get(input.user_id, &input.data.session).await {
            Ok(Some(selection)) => selection,
            result => {
                let key = if let Err(err) = result {
                    error!(%err, "Read quality selection error");
                    "quality.error"
                } else {
                    "quality.expired"
                };
                self.messenger
                    .answer_callback(AnswerCallbackRequest {
                        callback_id: input.callback_id,
                        text: Some(&t!(key, locale = input.locale.as_str())),
                    })
                    .await
                    .map_err(HandlerError::new)?;
                return Ok(());
            }
        };
        let locale = selection.job.chat_cfg.locale();
        let target = MenuMessage::ChatMessage {
            chat_id: input.chat_id,
            message_id: input.message_id,
        };
        match input.data.action {
            QualityAction::Page(page) if page <= selection.options.len().saturating_sub(1) / PAGE_SIZE => {
                let _ = self
                    .messenger
                    .answer_callback(AnswerCallbackRequest {
                        callback_id: input.callback_id,
                        text: None,
                    })
                    .await;
                let (text, keyboard) = render(&selection, &input.data.session, page, locale);
                self.messenger
                    .edit_menu(EditMenuRequest {
                        target,
                        text: &text,
                        keyboard: &keyboard,
                    })
                    .await
                    .map_err(HandlerError::new)?;
                return Ok(());
            }
            QualityAction::Best | QualityAction::Cancel => {}
            QualityAction::Video(index) if index < selection.options.len() => {}
            _ => return Ok(()),
        }
        let selection = match self.cache.take(input.user_id, &input.data.session).await {
            Ok(selection) => selection,
            Err(err) => {
                error!(%err, "Consume quality selection error");
                self.messenger
                    .answer_callback(AnswerCallbackRequest {
                        callback_id: input.callback_id,
                        text: Some(&t!("quality.error", locale = locale.as_str())),
                    })
                    .await
                    .map_err(HandlerError::new)?;
                return Ok(());
            }
        };
        let Some(mut selection) = selection else {
            self.messenger
                .answer_callback(AnswerCallbackRequest {
                    callback_id: input.callback_id,
                    text: Some(&t!("quality.expired", locale = locale.as_str())),
                })
                .await
                .map_err(HandlerError::new)?;
            return Ok(());
        };
        let _ = self
            .messenger
            .answer_callback(AnswerCallbackRequest {
                callback_id: input.callback_id,
                text: None,
            })
            .await;
        let text = if matches!(input.data.action, QualityAction::Cancel) {
            t!("quality.cancelled", locale = locale.as_str())
        } else {
            t!("download.preparing", locale = locale.as_str())
        };
        self.messenger
            .edit_menu(EditMenuRequest {
                target,
                text: &text,
                keyboard: &Keyboard { rows: vec![] },
            })
            .await
            .map_err(HandlerError::new)?;
        if matches!(input.data.action, QualityAction::Cancel) {
            return Ok(());
        }
        if let QualityAction::Video(index) = input.data.action {
            selection.job.video_quality = Some(selection.options[index].quality.clone());
        }
        selection.job.progress_message_id = Some(input.message_id);
        if let Err(err) = self.queue.enqueue(&selection.job).await {
            error!(%err, "Enqueue quality download error");
            self.messenger
                .edit_menu(EditMenuRequest {
                    target,
                    text: &progress::with_link(
                        &t!("download.error_queue", locale = locale.as_str()),
                        selection.job.url.as_ref().unwrap(),
                        Some(TextFormat::Html),
                    ),
                    keyboard: &Keyboard { rows: vec![] },
                })
                .await
                .map_err(HandlerError::new)?;
        }
        Ok(())
    }
}

fn quality_options(response: MediaInfoResponse) -> Option<(String, Vec<QualityOption>)> {
    if !response.video_quality_supported || response.entries.len() != 1 {
        return None;
    }
    let entry = response.entries.into_iter().next()?;
    let mut options: Vec<QualityOption> = vec![];
    for format in entry.formats {
        let Some(quality) = VideoQuality::new(format.width, format.height, format.fps) else {
            continue;
        };
        if options.iter().any(|option| option.quality == quality) {
            continue;
        }
        options.push(QualityOption {
            quality,
            filesize_approx: format.filesize_approx.filter(|size| *size > 0),
        });
    }
    options.sort_by(|a, b| {
        b.quality
            .width
            .saturating_mul(b.quality.height)
            .cmp(&a.quality.width.saturating_mul(a.quality.height))
            .then_with(|| b.quality.fps.unwrap_or_default().total_cmp(&a.quality.fps.unwrap_or_default()))
    });
    if options.is_empty() {
        return None;
    }
    Some((entry.title.unwrap_or(entry.id).chars().take(512).collect(), options))
}

fn render(selection: &QualitySelection, session: &str, page: usize, locale: Locale) -> (String, Keyboard) {
    let button = |text: String, action| Button {
        text,
        action: ButtonAction::Callback(
            SelectQuality {
                session: session.to_owned(),
                action,
            }
            .pack()
            .unwrap(),
        ),
    };
    let mut rows = vec![vec![button(
        t!("quality.best", locale = locale.as_str()).into_owned(),
        QualityAction::Best,
    )]];
    for (index, option) in selection.options.iter().enumerate().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
        let mut label = format!("{}×{}", option.quality.width, option.quality.height);
        if let Some(fps) = option.quality.fps {
            let _ = write!(label, " · {fps} fps");
        }
        if let Some(size) = option.filesize_approx {
            let _ = write!(label, " · ≈{} MB", size.div_ceil(1024 * 1024));
        }
        rows.push(vec![button(label, QualityAction::Video(index))]);
    }
    if page > 0 {
        rows.push(vec![button(
            t!("quality.previous", locale = locale.as_str()).into_owned(),
            QualityAction::Page(page - 1),
        )]);
    }
    if (page + 1) * PAGE_SIZE < selection.options.len() {
        rows.push(vec![button(
            t!("quality.next", locale = locale.as_str()).into_owned(),
            QualityAction::Page(page + 1),
        )]);
    }
    rows.push(vec![button(
        t!("quality.cancel", locale = locale.as_str()).into_owned(),
        QualityAction::Cancel,
    )]);
    (
        format!(
            "<b>{}</b>\n{}",
            html_quote(&selection.title),
            t!("quality.choose", locale = locale.as_str())
        ),
        Keyboard { rows },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::downloader::{MediaEntry, MediaFormatEntry};

    #[test]
    fn catalog_groups_real_dimensions_and_keeps_frame_rates_separate() {
        let format = |width, height, fps, size| MediaFormatEntry {
            width,
            height,
            fps,
            filesize_approx: size,
            ..Default::default()
        };
        let response = MediaInfoResponse {
            video_quality_supported: true,
            entries: vec![MediaEntry {
                title: Some("Synthetic video".into()),
                formats: vec![
                    format(Some(1276), Some(718), Some(59.94), Some(40)),
                    format(Some(1276), Some(718), Some(59.94), Some(20)),
                    format(Some(1276), Some(718), Some(30.0), None),
                    format(Some(640), Some(360), None, Some(0)),
                    format(None, Some(1080), Some(30.0), None),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(quality_options(MediaInfoResponse {
            video_quality_supported: false,
            ..response.clone()
        })
        .is_none());
        let (_, options) = quality_options(response).unwrap();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].quality, VideoQuality::new(Some(1276), Some(718), Some(59.94)).unwrap());
        assert_eq!(options[0].filesize_approx, Some(40));
        assert_eq!(options[1].quality.fps, Some(30.0));
        assert!(options[2].filesize_approx.is_none());
    }
}
