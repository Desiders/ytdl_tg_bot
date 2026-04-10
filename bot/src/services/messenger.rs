pub mod telegram;

use crate::entities::MediaForUpload;

use std::future::Future;
use telers::errors::HandlerError;
use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextFormat {
    Html,
}

#[derive(Debug, thiserror::Error)]
#[error("Messenger error: {message}")]
pub struct MessengerError {
    message: Box<str>,
}

impl MessengerError {
    pub fn new(message: impl Into<Box<str>>) -> Self {
        Self { message: message.into() }
    }
}

impl From<MessengerError> for HandlerError {
    fn from(value: MessengerError) -> Self {
        Self::new(value)
    }
}

pub struct SentMessage {
    pub message_id: i64,
}

pub struct SendTextRequest<'a> {
    pub chat_id: i64,
    pub text: &'a str,
    pub reply_to_message_id: Option<i64>,
    pub format: Option<TextFormat>,
    pub disable_link_preview: bool,
}

pub enum EditTarget<'a> {
    ChatMessage { chat_id: i64, message_id: i64 },
    InlineMessage { inline_message_id: &'a str },
}

pub struct EditTextRequest<'a> {
    pub target: EditTarget<'a>,
    pub text: &'a str,
    pub format: Option<TextFormat>,
    pub disable_link_preview: bool,
    pub clear_inline_keyboard: bool,
}

pub struct DeleteMessageRequest {
    pub chat_id: i64,
    pub message_id: i64,
}

pub struct AnswerInlineErrorRequest<'a> {
    pub query_id: &'a str,
    pub text: &'a str,
}

pub struct InlineQueryArticle {
    pub id: String,
    pub title: String,
    pub content_text: String,
    pub content_format: Option<TextFormat>,
    pub thumbnail_url: Option<String>,
    pub description: Option<String>,
    pub callback_data: Option<String>,
}

pub struct AnswerInlineQueryRequest<'a> {
    pub query_id: &'a str,
    pub results: Vec<InlineQueryArticle>,
    pub cache_time: i64,
    pub is_personal: bool,
}

pub struct UploadVideoRequest<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub media_for_upload: MediaForUpload,
    pub name: &'a str,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration: Option<i64>,
    pub with_delete: bool,
    pub webpage_url: &'a Url,
    pub link_is_visible: bool,
}

pub struct UploadAudioRequest<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub media_for_upload: MediaForUpload,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub performer: Option<&'a str>,
    pub duration: Option<i64>,
    pub with_delete: bool,
    pub webpage_url: &'a Url,
    pub link_is_visible: bool,
}

pub struct SendMediaByIdRequest<'a> {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub remote_id: &'a str,
    pub webpage_url: Option<&'a Url>,
    pub link_is_visible: bool,
}

pub struct EditMediaByIdRequest<'a> {
    pub inline_message_id: &'a str,
    pub remote_id: &'a str,
    pub webpage_url: Option<&'a Url>,
    pub link_is_visible: bool,
}

pub struct MediaGroupItem {
    pub remote_id: Box<str>,
    pub webpage_url: Option<Url>,
}

pub struct SendMediaGroupRequest {
    pub chat_id: i64,
    pub reply_to_message_id: Option<i64>,
    pub items: Vec<MediaGroupItem>,
    pub link_is_visible: bool,
}

pub trait MessengerPort: Send + Sync {
    fn username(&self) -> impl Future<Output = Result<String, MessengerError>> + Send;

    fn send_text(&self, request: SendTextRequest<'_>) -> impl Future<Output = Result<SentMessage, MessengerError>> + Send;

    fn edit_text(&self, request: EditTextRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn delete_message(&self, request: DeleteMessageRequest) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn answer_inline_error(&self, request: AnswerInlineErrorRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn answer_inline_query(&self, request: AnswerInlineQueryRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn upload_video(&self, request: UploadVideoRequest<'_>) -> impl Future<Output = Result<Box<str>, MessengerError>> + Send;

    fn upload_audio(&self, request: UploadAudioRequest<'_>) -> impl Future<Output = Result<Box<str>, MessengerError>> + Send;

    fn send_video_by_id(&self, request: SendMediaByIdRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn send_audio_by_id(&self, request: SendMediaByIdRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn edit_video_by_id(&self, request: EditMediaByIdRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn edit_audio_by_id(&self, request: EditMediaByIdRequest<'_>) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn send_video_group(&self, request: SendMediaGroupRequest) -> impl Future<Output = Result<(), MessengerError>> + Send;

    fn send_audio_group(&self, request: SendMediaGroupRequest) -> impl Future<Output = Result<(), MessengerError>> + Send;
}
