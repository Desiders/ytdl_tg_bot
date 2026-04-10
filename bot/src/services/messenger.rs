pub mod telegram;

use telers::errors::HandlerError;

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

pub trait MessengerPort: Send + Sync {
    async fn username(&self) -> Result<String, MessengerError>;

    async fn send_text(&self, request: SendTextRequest<'_>) -> Result<SentMessage, MessengerError>;

    async fn edit_text(&self, request: EditTextRequest<'_>) -> Result<(), MessengerError>;

    async fn delete_message(&self, request: DeleteMessageRequest) -> Result<(), MessengerError>;

    async fn answer_inline_error(&self, request: AnswerInlineErrorRequest<'_>) -> Result<(), MessengerError>;

    async fn answer_inline_query(&self, request: AnswerInlineQueryRequest<'_>) -> Result<(), MessengerError>;
}
