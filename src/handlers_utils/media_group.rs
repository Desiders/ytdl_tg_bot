use std::mem;
use telers::{
    errors::SessionErrorKind,
    methods::SendMediaGroup,
    types::{ChatIdKind, InputMedia, Message},
    Bot,
};

pub async fn send_from_input_media_list(
    bot: &Bot,
    chat_id: impl Into<ChatIdKind>,
    input_media_list: Vec<impl Into<InputMedia<'_>>>,
    reply_to_message_id: Option<i64>,
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
                bot.send(
                    SendMediaGroup::new(chat_id.clone(), media_group)
                        .reply_to_message_id_option(reply_to_message_id)
                        .allow_sending_without_reply(true),
                )
                .await?,
            );

            cur_media_group_len = 0;
        }
    }

    if cur_media_group_len != 0 {
        messages.extend(
            bot.send(
                SendMediaGroup::new(chat_id.clone(), cur_media_group)
                    .reply_to_message_id_option(reply_to_message_id)
                    .allow_sending_without_reply(true),
            )
            .await?,
        );
    }

    Ok(messages.into_boxed_slice())
}
