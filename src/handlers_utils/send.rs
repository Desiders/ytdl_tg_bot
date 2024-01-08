use backoff::{backoff::Backoff as _, ExponentialBackoff};
use std::mem;
use telers::{
    errors::{SessionErrorKind, TelegramErrorKind},
    methods::{SendMediaGroup, TelegramMethod},
    types::{ChatIdKind, InputMedia, Message, ReplyParameters},
    Bot,
};
use tracing::{event, instrument, Level};

const MAX_RETRIES: usize = 3;

/// Sends a request to the Telegram Bot API with limited retries.
/// # Arguments
/// * `bot` - Bot instance
/// * `method` - Method to send
/// * `request_timeout` - Request timeout
/// # Notes
/// This function will retry the request if the following serrors occur:
/// - [`SessionErrorKind::Client`]
/// - [`TelegramErrorKind::NetworkError`]
/// - [`TelegramErrorKind::RetryAfter`]
/// - [`TelegramErrorKind::ServerError`]
/// - [`TelegramErrorKind::RestartingTelegram`]
/// # Returns
/// - `Ok(T::Return)` - If the request was successful
/// - `Err(SessionErrorKind)` - If the request was unsuccessful and the maximum number of retries was exceeded
#[instrument(skip_all)]
pub async fn with_retries<T, TRef>(bot: &Bot, method: TRef, request_timeout: Option<f32>) -> Result<T::Return, SessionErrorKind>
where
    T: TelegramMethod + Send + Sync,
    T::Method: Send + Sync,
    TRef: AsRef<T> + Clone,
{
    let mut backoff = ExponentialBackoff::default();
    let mut cur_retry_count = 0;

    loop {
        match if let Some(request_timeout) = request_timeout {
            bot.send_with_timeout(method.clone(), request_timeout).await
        } else {
            bot.send(method.clone()).await
        } {
            Ok(result) => break Ok(result),
            Err(err) => {
                cur_retry_count += 1;

                if cur_retry_count > MAX_RETRIES {
                    event!(Level::ERROR, "Max retries exceeded");

                    break Err(err);
                } else {
                    event!(Level::WARN, "Retrying request {}/{}", cur_retry_count, MAX_RETRIES);
                }

                if let Some(duration) = backoff.next_backoff() {
                    event!(Level::DEBUG, "Sleeping for {duration:?} seconds");

                    tokio::time::sleep(duration).await;
                }

                match err {
                    SessionErrorKind::Client(_)
                    | SessionErrorKind::Telegram(TelegramErrorKind::NetworkError { .. })
                    | SessionErrorKind::Telegram(TelegramErrorKind::RetryAfter { .. })
                    | SessionErrorKind::Telegram(TelegramErrorKind::ServerError { .. })
                    | SessionErrorKind::Telegram(TelegramErrorKind::RestartingTelegram { .. }) => {}
                    // We don't want to retry on these errors
                    _ => {
                        event!(Level::ERROR, "Unexpected error: {err:?}");

                        break Err(err);
                    }
                }
            }
        }
    }
}

/// Sends a media groups to the Telegram Bot API with limited retries for each media group.
/// # Arguments
/// * `bot` - Bot instance
/// * `chat_id` - Chat ID
/// * `input_media_list` - List of input media
/// * `reply_to_message_id` - If the message is a reply, ID of the original message
/// * `request_timeout` - Request timeout
/// # Notess
/// If the number of input media is greater than 10, the function will split the input media into groups,
/// each of which will contain no more than 10 input media, and send them separately.
///
/// This function will retry the request if the error occurs, see [`with_retries`] for more info.
#[instrument(skip_all)]
pub async fn media_groups(
    bot: &Bot,
    chat_id: impl Into<ChatIdKind>,
    input_media_list: Vec<impl Into<InputMedia<'_>>>,
    reply_to_message_id: Option<i64>,
    request_timeout: Option<f32>,
) -> Result<Box<[Message]>, SessionErrorKind> {
    let chat_id = chat_id.into();
    let input_media_len = input_media_list.len();

    if input_media_len == 0 {
        return Ok(Box::new([]));
    }

    let cap = if input_media_len > 10 { 10 } else { input_media_len };

    let mut messages = Vec::with_capacity(input_media_len);

    let mut cur_media_group = Vec::with_capacity(cap);
    let mut cur_media_group_len = 0;

    for input_media in input_media_list {
        let input_media = input_media.into();

        cur_media_group.push(input_media);
        cur_media_group_len += 1;

        if cur_media_group_len == 10 {
            let media_group = mem::take(&mut cur_media_group);

            messages.extend(
                with_retries(
                    bot,
                    SendMediaGroup::new(chat_id.clone(), media_group).reply_parameters_option(
                        reply_to_message_id
                            .map(|reply_to_message_id| ReplyParameters::new(reply_to_message_id).allow_sending_without_reply(true)),
                    ),
                    request_timeout,
                )
                .await?,
            );

            cur_media_group_len = 0;
        }
    }

    if cur_media_group_len != 0 {
        messages.extend(
            with_retries(
                bot,
                SendMediaGroup::new(chat_id.clone(), cur_media_group).reply_parameters_option(
                    reply_to_message_id
                        .map(|reply_to_message_id| ReplyParameters::new(reply_to_message_id).allow_sending_without_reply(true)),
                ),
                request_timeout,
            )
            .await?,
        );
    }

    Ok(messages.into_boxed_slice())
}
