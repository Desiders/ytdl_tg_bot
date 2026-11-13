use std::{fmt::Write as _, str::FromStr as _, sync::Arc, time::Duration};

use proto::downloader::{MediaInfoRequest, MediaInfoResponse};
use rust_i18n::t;
use telers::{callback_data::CallbackData as _, errors::HandlerError, utils::text::html_quote};
use time::OffsetDateTime;
use tokio::time::timeout;
use tracing::{error, instrument};
use url::Url;
use uuid::Uuid;

use crate::{
    entities::{language::Language, ChatConfig, DownloadJob, JobTarget, MediaCatalog, MediaCatalogFormat, MediaFormat, Params, Range},
    handlers_utils::progress,
    interactors::Interactor,
    locale::Locale,
    services::{
        media_catalog_cache::MediaCatalogCache,
        messenger::{AnswerCallbackRequest, Button, ButtonAction, EditMenuRequest, Keyboard, MenuMessage, MessengerPort, TextFormat},
        node_router::{get_media_info, NodeRouter},
        quality_selection_cache::{QualitySelection, QualitySelectionCache},
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
    catalogs: Arc<MediaCatalogCache>,
    error_formatter: Arc<ErrorFormatter>,
}

impl<Messenger> Open<Messenger> {
    pub const fn new(
        messenger: Arc<Messenger>,
        node_router: Arc<NodeRouter>,
        cache: Arc<QualitySelectionCache>,
        catalogs: Arc<MediaCatalogCache>,
        error_formatter: Arc<ErrorFormatter>,
    ) -> Self {
        Self {
            messenger,
            node_router,
            cache,
            catalogs,
            error_formatter,
        }
    }
}

pub struct OpenInput<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    pub is_private: bool,
    pub progress_message_id: Option<i64>,
    pub force_refresh: bool,
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
        let message_id = if let Some(message_id) = input.progress_message_id {
            message_id
        } else {
            progress::new(
                self.messenger.as_ref(),
                &t!("download.preparing", locale = locale.as_str()),
                input.chat_id,
                Some(input.message_id),
                None,
            )
            .await
            .map_err(HandlerError::new)?
            .message_id
        };
        let target = MenuMessage::ChatMessage {
            chat_id: input.chat_id,
            message_id,
        };
        let language = input
            .params
            .0
            .get("lang")
            .map_or_else(Language::default, |lang| Language::from_str(lang).unwrap());
        let catalog = match self
            .load_catalog(url, &language, input.force_refresh || input.params.get_bool("overwrite"))
            .await
        {
            Ok(Some(catalog)) => catalog,
            result => {
                let key = if let Err(err) = result {
                    error!(%err, "Get quality options error");
                    "quality.error"
                } else {
                    "quality.unavailable"
                };
                self.messenger
                    .edit_menu(EditMenuRequest {
                        target,
                        text: &progress::with_link(&t!(key, locale = locale.as_str()), url, Some(TextFormat::Html)),
                        keyboard: &Keyboard { rows: vec![] },
                    })
                    .await
                    .map_err(HandlerError::new)?;
                return Ok(());
            }
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
            catalog_id: catalog.id,
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
        let (text, keyboard) = render_resolutions(&catalog, &session, 0, locale);
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

impl<Messenger> Open<Messenger> {
    async fn load_catalog(&self, url: &Url, language: &Language, overwrite: bool) -> Result<Option<MediaCatalog>, String> {
        let max_file_size = i64::try_from(self.node_router.max_file_size()).unwrap_or(i64::MAX);
        let audio_language = language.language.as_deref().unwrap_or_default();
        if !overwrite {
            if let Some(catalog) = self
                .catalogs
                .get(url.as_str(), audio_language, max_file_size)
                .await
                .map_err(|err| err.to_string())?
            {
                return Ok(Some(catalog));
            }
        }
        let response = timeout(
            Duration::from_secs(180),
            get_media_info(
                &self.node_router,
                url.domain(),
                MediaInfoRequest {
                    url: url.to_string(),
                    audio_language: audio_language.to_owned(),
                    playlist_range: Some(Range::default().into()),
                    media_type: "video".to_owned(),
                    max_file_size: self.node_router.max_file_size(),
                    list_video_formats: true,
                    ..Default::default()
                },
            ),
        )
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| self.error_formatter.format(&err).into_owned())?;
        let Some(catalog) = catalog_from_response(response, url, audio_language, max_file_size) else {
            return Ok(None);
        };
        self.catalogs.set(catalog).await.map(Some).map_err(|err| err.to_string())
    }
}

pub struct Select<Messenger> {
    messenger: Arc<Messenger>,
    cache: Arc<QualitySelectionCache>,
    catalogs: Arc<MediaCatalogCache>,
    open: Arc<Open<Messenger>>,
    queue: Arc<RedisJobQueue>,
}

impl<Messenger> Select<Messenger> {
    pub const fn new(
        messenger: Arc<Messenger>,
        cache: Arc<QualitySelectionCache>,
        catalogs: Arc<MediaCatalogCache>,
        open: Arc<Open<Messenger>>,
        queue: Arc<RedisJobQueue>,
    ) -> Self {
        Self {
            messenger,
            cache,
            catalogs,
            open,
            queue,
        }
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
        let catalog = match self.catalogs.get_catalog(selection.catalog_id).await {
            Ok(Some(catalog)) => catalog,
            Ok(None) => {
                let _ = self
                    .messenger
                    .answer_callback(AnswerCallbackRequest {
                        callback_id: input.callback_id,
                        text: None,
                    })
                    .await;
                let Some(selection) = self
                    .cache
                    .take(input.user_id, &input.data.session)
                    .await
                    .map_err(HandlerError::new)?
                else {
                    return Ok(());
                };
                let JobTarget::Command { message_id, .. } = selection.job.target else {
                    unreachable!()
                };
                Box::pin(self.open.execute(OpenInput {
                    chat_id: input.chat_id,
                    message_id,
                    is_private: true,
                    progress_message_id: Some(input.message_id),
                    force_refresh: true,
                    url: selection.job.url.as_ref(),
                    params: &selection.job.params,
                    chat_cfg: &selection.job.chat_cfg,
                }))
                .await?;
                return Ok(());
            }
            result => {
                let key = if let Err(err) = result {
                    error!(%err, "Read quality catalog error");
                    "quality.error"
                } else {
                    "quality.expired"
                };
                self.messenger
                    .answer_callback(AnswerCallbackRequest {
                        callback_id: input.callback_id,
                        text: Some(&t!(key, locale = locale.as_str())),
                    })
                    .await
                    .map_err(HandlerError::new)?;
                return Ok(());
            }
        };
        let groups = resolutions(&catalog);
        let screen = match input.data.action {
            QualityAction::Page(page) if page <= groups.len().saturating_sub(1) / PAGE_SIZE => {
                Some(render_resolutions(&catalog, &input.data.session, page, locale))
            }
            QualityAction::Resolution(id) => Some(if let Some(index) = resolution_position(&catalog, id) {
                render_formats(&catalog, &input.data.session, index, 0, locale)
            } else {
                render_resolutions(&catalog, &input.data.session, 0, locale)
            }),
            QualityAction::FormatsPage { resolution, page } => {
                let screen = resolution_position(&catalog, resolution)
                    .filter(|index| page <= formats_at_resolution(&catalog, groups[*index]).len().saturating_sub(1) / PAGE_SIZE)
                    .map_or_else(
                        || render_resolutions(&catalog, &input.data.session, 0, locale),
                        |index| render_formats(&catalog, &input.data.session, index, page, locale),
                    );
                Some(screen)
            }
            QualityAction::Best | QualityAction::Cancel => None,
            QualityAction::Video(id) if catalog.formats.iter().any(|format| format.id == id) => None,
            QualityAction::Video(_) => Some(render_resolutions(&catalog, &input.data.session, 0, locale)),
            QualityAction::Page(_) => return Ok(()),
        };
        if let Some((text, keyboard)) = screen {
            let _ = self
                .messenger
                .answer_callback(AnswerCallbackRequest {
                    callback_id: input.callback_id,
                    text: None,
                })
                .await;
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
            selection.job.video_quality = catalog
                .formats
                .iter()
                .find(|format| format.id == index)
                .unwrap()
                .format
                .video_quality();
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

fn catalog_from_response(response: MediaInfoResponse, url: &Url, audio_language: &str, max_file_size: i64) -> Option<MediaCatalog> {
    if !response.video_format_selection_supported || response.entries.len() != 1 {
        return None;
    }
    let entry = response.entries.into_iter().next()?;
    let mut formats: Vec<MediaCatalogFormat> = vec![];
    for format in entry.formats {
        if format.format_id.is_empty() || VideoQuality::new(format.width, format.height, format.fps).is_none() {
            continue;
        }
        if formats.iter().any(|existing| existing.format.format_id == format.format_id) {
            continue;
        }
        formats.push(MediaCatalogFormat {
            id: 0,
            format: MediaFormat {
                format_id: format.format_id,
                format_note: None,
                ext: format.ext,
                width: format.width,
                height: format.height,
                fps: format.fps,
                vcodec: format.vcodec,
                acodec: format.acodec,
                aspect_ratio: format.aspect_ratio,
                filesize_approx: format.filesize_approx.filter(|size| *size > 0),
            },
        });
    }
    if formats.is_empty() {
        return None;
    }
    Some(MediaCatalog {
        id: Uuid::now_v7(),
        url: url.to_string(),
        audio_language: audio_language.to_owned(),
        max_file_size,
        media_id: entry.id.clone(),
        webpage_url: entry.webpage_url,
        title: entry.title.unwrap_or(entry.id),
        description: entry.description,
        duration: entry.duration,
        updated_at: OffsetDateTime::now_utc(),
        formats,
    })
}

fn resolutions(catalog: &MediaCatalog) -> Vec<(i64, i64)> {
    let mut resolutions = vec![];
    for entry in &catalog.formats {
        let format = &entry.format;
        let resolution = (format.width.unwrap(), format.height.unwrap());
        if !resolutions.contains(&resolution) {
            resolutions.push(resolution);
        }
    }
    resolutions.sort_by_key(|(width, height)| width.saturating_mul(*height));
    resolutions.reverse();
    resolutions
}

fn formats_at_resolution(catalog: &MediaCatalog, resolution: (i64, i64)) -> Vec<&MediaCatalogFormat> {
    catalog
        .formats
        .iter()
        .filter(|entry| (entry.format.width.unwrap(), entry.format.height.unwrap()) == resolution)
        .collect()
}

fn resolution_position(catalog: &MediaCatalog, id: i64) -> Option<usize> {
    let format = &catalog.formats.iter().find(|format| format.id == id)?.format;
    resolutions(catalog)
        .iter()
        .position(|resolution| *resolution == (format.width.unwrap(), format.height.unwrap()))
}

fn button(text: String, session: &str, action: QualityAction) -> Vec<Button> {
    vec![Button {
        text,
        action: ButtonAction::Callback(
            SelectQuality {
                session: session.to_owned(),
                action,
            }
            .pack()
            .unwrap(),
        ),
    }]
}

fn menu_text(catalog: &MediaCatalog, prompt: &str) -> String {
    let title: String = catalog.title.chars().take(512).collect();
    format!("<b>{}</b>\n{prompt}", html_quote(&title))
}

fn render_resolutions(catalog: &MediaCatalog, session: &str, page: usize, locale: Locale) -> (String, Keyboard) {
    let groups = resolutions(catalog);
    let mut rows = vec![button(
        t!("quality.best", locale = locale.as_str()).into_owned(),
        session,
        QualityAction::Best,
    )];
    for (width, height) in groups.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
        rows.push(button(
            format!("{width}×{height}"),
            session,
            QualityAction::Resolution(formats_at_resolution(catalog, (*width, *height))[0].id),
        ));
    }
    if page > 0 {
        rows.push(button(
            t!("quality.previous", locale = locale.as_str()).into_owned(),
            session,
            QualityAction::Page(page - 1),
        ));
    }
    if (page + 1) * PAGE_SIZE < groups.len() {
        rows.push(button(
            t!("quality.next", locale = locale.as_str()).into_owned(),
            session,
            QualityAction::Page(page + 1),
        ));
    }
    rows.push(button(
        t!("quality.cancel", locale = locale.as_str()).into_owned(),
        session,
        QualityAction::Cancel,
    ));
    (
        menu_text(catalog, &t!("quality.choose_resolution", locale = locale.as_str())),
        Keyboard { rows },
    )
}

fn render_formats(catalog: &MediaCatalog, session: &str, resolution: usize, page: usize, locale: Locale) -> (String, Keyboard) {
    let group = resolutions(catalog)[resolution];
    let formats = formats_at_resolution(catalog, group);
    let mut rows = vec![];
    for entry in formats.iter().skip(page * PAGE_SIZE).take(PAGE_SIZE) {
        let format = &entry.format;
        let mut label = format.ext.to_uppercase();
        if let Some(codec) = &format.vcodec {
            let _ = write!(label, " · {codec}");
        }
        if let Some(codec) = &format.acodec {
            if codec != "none" {
                let _ = write!(label, " + {codec}");
            }
        }
        if let Some(fps) = format.fps {
            let _ = write!(label, " · {fps} fps");
        }
        if let Some(size) = format.filesize_approx {
            let _ = write!(label, " · ≈{} MB", size.div_ceil(1024 * 1024));
        }
        rows.push(button(label, session, QualityAction::Video(entry.id)));
    }
    if page > 0 {
        rows.push(button(
            t!("quality.previous", locale = locale.as_str()).into_owned(),
            session,
            QualityAction::FormatsPage {
                resolution: formats[0].id,
                page: page - 1,
            },
        ));
    }
    if (page + 1) * PAGE_SIZE < formats.len() {
        rows.push(button(
            t!("quality.next", locale = locale.as_str()).into_owned(),
            session,
            QualityAction::FormatsPage {
                resolution: formats[0].id,
                page: page + 1,
            },
        ));
    }
    rows.push(button(
        t!("quality.back", locale = locale.as_str()).into_owned(),
        session,
        QualityAction::Page(resolution / PAGE_SIZE),
    ));
    rows.push(button(
        t!("quality.cancel", locale = locale.as_str()).into_owned(),
        session,
        QualityAction::Cancel,
    ));
    (
        menu_text(
            catalog,
            &format!(
                "{} · {}×{}",
                t!("quality.choose_format", locale = locale.as_str()),
                group.0,
                group.1
            ),
        ),
        Keyboard { rows },
    )
}
