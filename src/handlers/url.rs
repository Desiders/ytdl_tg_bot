use crate::{
    extractors::YtDlpWrapper,
    models::{CombinedFormats, FileIds, Videos},
};

use std::{mem, path::PathBuf, sync::Arc, thread};
use telers::{
    enums::ParseMode,
    errors::{HandlerError, SessionErrorKind},
    event::{telegram::HandlerResult, EventReturn},
    methods::{SendMediaGroup, SendMessage},
    types::{InputFile, InputMediaVideo, Message},
    utils::text_decorations::{TextDecoration, HTML_DECORATION},
    Bot,
};
use tempdir::TempDir;
use tokio::task::JoinHandle;
use tracing::{event, Level};
use youtube_dl::YoutubeDl;

const MEGABYTE: u64 = 1_000_000;

fn ytdl_new_with_get_info_args(url: &str, ytdl_path: &str) -> YoutubeDl {
    let mut ytdl = YoutubeDl::new(url);
    ytdl.socket_timeout("15");
    ytdl.extra_arg("--no-call-home");
    ytdl.extra_arg("--no-check-certificate");
    ytdl.extra_arg("--skip-download");
    ytdl.extra_arg("--abort-on-error");
    ytdl.youtube_dl_path(ytdl_path);

    ytdl
}

fn ytdl_new_with_download_args(id_or_url: &str, ytdl_path: &str, format: &str) -> YoutubeDl {
    let mut ytdl = YoutubeDl::new(id_or_url);
    ytdl.socket_timeout("15");
    ytdl.output_template("%(id)s.%(ext)s");
    ytdl.extra_arg("--no-call-home");
    ytdl.extra_arg("--no-check-certificate");
    ytdl.extra_arg("--no-cache-dir");
    ytdl.extra_arg("--no-mtime");
    ytdl.extra_arg("--abort-on-error");
    ytdl.youtube_dl_path(ytdl_path);
    ytdl.format(format);

    ytdl
}

async fn await_handles(handles: Vec<JoinHandle<HandlerResult>>) -> HandlerResult {
    for handle in handles {
        match handle.await {
            Ok(result) => result?,
            Err(err) => {
                event!(Level::ERROR, %err, "Error while joining handle");

                return Err(HandlerError::new(err));
            }
        };

        event!(Level::DEBUG, "Handle joined");
    }

    event!(Level::DEBUG, "All handles joined");

    Ok(EventReturn::Finish)
}

fn spawn_delete_file(path: PathBuf) {
    thread::spawn(move || {
        if let Err(err) = std::fs::remove_file(&path) {
            event!(Level::ERROR, %err, ?path, "Error while deleting file");
        }
    });
}

#[allow(clippy::cast_precision_loss)]
fn get_request_timeout_by_len(len: usize) -> f32 {
    (len * 90) as f32
}

async fn send_videos_and_get_file_ids(
    bot: &Bot,
    message: &Message,
    media_group: Vec<InputMediaVideo<'_>>,
    request_timeout: f32,
) -> Result<Vec<FileIds>, SessionErrorKind> {
    bot.send(
        &SendMediaGroup::new(message.chat_id(), media_group)
            .reply_to_message_id(message.message_id)
            .allow_sending_without_reply(true),
        Some(request_timeout),
    )
    .await?
    .into_iter()
    .map(|message| {
        let video = message.video.unwrap();

        Ok(FileIds {
            file_id: video.file_id,
            file_unique_id: video.file_unique_id,
        })
    })
    .collect()
}

#[allow(clippy::too_many_lines)]
pub async fn url(
    bot: Arc<Bot>,
    message: Message,
    YtDlpWrapper(yt_dlp_config): YtDlpWrapper,
) -> HandlerResult {
    // `unwrap` is safe here, because we check that `message.text` is `Some` by filters
    let url = message.text.as_ref().unwrap();
    let chat_id = message.chat_id();

    let ytdl = ytdl_new_with_get_info_args(url, &yt_dlp_config.full_path);

    let videos = match ytdl.run_async().await {
        Ok(ytdl_output) => Videos::from(ytdl_output),
        Err(err) => {
            event!(Level::ERROR, %err, url, "Error while getting video/playlist info");

            bot.send(
                &SendMessage::new(
                    chat_id,
                    "Sorry, an error occurred while getting video/playlist info. Try again later.",
                )
                .reply_to_message_id(message.message_id)
                .allow_sending_without_reply(true),
                None,
            )
            .await?;

            return Ok(EventReturn::Finish);
        }
    };

    if videos.is_empty() {
        event!(Level::ERROR, url, "Playlist doesn't have entries");

        bot.send(
            &SendMessage::new(chat_id, "Playlist doesn't have videos to download.")
                .reply_to_message_id(message.message_id)
                .allow_sending_without_reply(true),
            None,
        )
        .await?;

        return Ok(EventReturn::Finish);
    }

    // Create temp dir to store videos and playlists
    let temp_dir = TempDir::new("ytdl_videos").map_err(HandlerError::new)?;
    // `unwrap` is safe here, because `TempDir` creates temp dir with ASCII name, so it's always valid UTF-8
    let temp_dir_path = temp_dir.path().to_str().unwrap().to_owned();

    // Max files size to download in one request.
    // We use this value to choose the best format for each video and avoid errors from Telegram.
    let max_files_size_in_bytes = yt_dlp_config.max_files_size_in_bytes;
    let max_files_size_in_mb = max_files_size_in_bytes / MEGABYTE;

    // Create handles to download videos in tokio tasks and wait for them
    await_handles(videos.clone().map(|video| {
        let bot = bot.clone();
        let yt_dlp_config = yt_dlp_config.clone();
        let temp_dir_path = temp_dir_path.clone();

        tokio::spawn(async move {
            let combined_formats = CombinedFormats::try_from(video.formats.as_ref().map(AsRef::as_ref)).map(|mut combined_formats| {
                // Filter out formats that are bigger than `max_files_size_in_bytes`
                combined_formats.skip_with_size_less_than(max_files_size_in_bytes);
                combined_formats
            });
            let combined_formats_is_err_or_empty = combined_formats.as_ref().map_or(true, CombinedFormats::is_empty);

            let title = video.title.as_ref().map_or("Empty title", String::as_str);

            if combined_formats_is_err_or_empty {
                event!(Level::ERROR, ?video, "No combined formats found");

                bot.send(
                    &SendMessage::new(
                        chat_id,
                        format!(
                            "Sorry, suitable formats for video `{title}` not found. \
                            Maybe video size is too big or video has unsupported format.",
                            title = HTML_DECORATION.code(HTML_DECORATION.quote(title).as_str()),
                        ),
                    ).parse_mode(ParseMode::HTML),
                    None,
                )
                .await?;

                return Ok(EventReturn::Finish);
            }

            // `unwrap` is safe here, because we check that `combined_formats` is `Ok` and not empty
            let combined_formats = combined_formats.unwrap();
            let id = video.id.as_str();

            let mut is_downloaded = false;

            for ref combined_format in combined_formats {
                // `unwrap` is safe here, because we create `combined_formats` from `video.formats` and check that it's not empty
                let format_id = combined_format.get_format_id().unwrap();

                let ytdl = ytdl_new_with_download_args(id, yt_dlp_config.full_path.as_str(), format_id.as_str());

                if let Err(err) = ytdl.download_to_async(temp_dir_path.as_str()).await {
                    event!(Level::ERROR, %err, ?video, ?combined_format, "Error while downloading video");

                    continue;
                };

                is_downloaded = true;

                break;
            }

            // If video is downloaded, we don't need to download other formats for this video
            if is_downloaded {
                return Ok(EventReturn::Finish);
            }

            bot.send(
                &SendMessage::new(
                    chat_id,
                    format!(
                        "Sorry, an error occurred while downloading video `{title}`. \
                        Try again later.",
                        title = HTML_DECORATION.code(HTML_DECORATION.quote(title).as_str()),
                    ),
                )
                .reply_to_message_id(message.message_id)
                .allow_sending_without_reply(true)
                .parse_mode(ParseMode::HTML),
                None,
            )
            .await?;

            Ok(EventReturn::Finish)
        })
    }).collect()).await?;

    // Read temp dir to get video files and send them to the chat
    let mut stream = tokio::fs::read_dir(temp_dir_path).await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while reading temp playlist dir");

        HandlerError::new(err)
    })?;

    let mut count_files_skipped_by_size: u16 = 0;

    let (mut input_media, mut input_media_sizes) = (vec![], vec![]);

    while let Some(file) = stream.next_entry().await.map_err(|err| {
        event!(Level::ERROR, %err, "Error while reading temp playlist dir entry");

        HandlerError::new(err)
    })? {
        let file_path = file.path();

        let metadata = match file.metadata().await {
            Ok(metadata) => metadata,
            Err(err) => {
                event!(Level::ERROR, %err, ?file_path, "Error while reading file metadata");

                spawn_delete_file(file_path);

                bot.send(
                    &SendMessage::new(
                        chat_id,
                        "Sorry, something went wrong while reading file metadata. File skipped to avoid errors.",
                    )
                    .reply_to_message_id(message.message_id)
                    .allow_sending_without_reply(true),
                    None,
                )
                .await?;

                continue;
            }
        };

        let file_id = file.file_name();
        let file_name = file_id.to_string_lossy();
        let file_name_without_ext = file_name
            .split('.')
            .next()
            .expect("File name doesn't have extension");
        let file_size_in_bytes = metadata.len();
        let file_size_in_mb = file_size_in_bytes / MEGABYTE;

        if file_size_in_bytes > max_files_size_in_bytes {
            event!(
                Level::DEBUG,
                path = ?file_path,
                file_size_in_bytes,
                file_size_in_mb,
                "File size is too big, skipping"
            );

            spawn_delete_file(file_path);

            count_files_skipped_by_size += 1;

            bot.send(
                &SendMessage::new(
                    chat_id,
                    format!(
                        "Sorry, file size is too big to send. \
                         Max file size is {max_files_size_in_mb} MB. \
                         Current file size is {file_size_in_mb} MB. \
                         File skipped to avoid errors.",
                    ),
                ),
                None,
            )
            .await?;

            continue;
        }

        event!(Level::DEBUG, path = ?file_path, file_size_in_bytes, file_size_in_mb, "Adding file to playlist");

        let video = videos
            .get_by_id(file_name_without_ext)
            .expect("Video not found by id");

        input_media.push(if let Some(thumbnail) = video.get_best_thumbnail() {
            InputMediaVideo::new(InputFile::fs(file_path, None))
                .supports_streaming(true)
                .thumb(InputFile::url(thumbnail.url.as_ref().unwrap()))
        } else {
            InputMediaVideo::new(InputFile::fs(file_path, None)).supports_streaming(true)
        });
        input_media_sizes.push(file_size_in_bytes);
    }

    if input_media.is_empty() {
        event!(Level::ERROR, url, "No videos found in playlist");

        // If we skip some videos because of their size, we don't need to send a message about download failure again
        if count_files_skipped_by_size > 0 {
            return Ok(EventReturn::Finish);
        }

        bot.send(
            &SendMessage::new(
                chat_id,
                "Sorry, something went wrong. Not any video has been uploaded. Try again later.",
            )
            .reply_to_message_id(message.message_id)
            .allow_sending_without_reply(true),
            None,
        )
        .await?;

        return Ok(EventReturn::Finish);
    }

    event!(Level::DEBUG, ?input_media, "Sending playlist");

    let mut media_group_file_size = 0;
    let mut media_group = vec![];
    let mut file_ids = vec![];

    for (input_media, input_media_size) in input_media.into_iter().zip(input_media_sizes) {
        if (media_group_file_size + input_media_size) > max_files_size_in_bytes {
            let mut media_group_to_send = vec![];
            mem::swap(&mut media_group_to_send, &mut media_group);

            // This should never happen, because we check each video size when reading playlist dir
            assert!(
                !media_group_to_send.is_empty(),
                "Media group file size is reached, but media group is empty"
            );

            media_group_file_size = 0;

            let request_timeout = get_request_timeout_by_len(media_group_to_send.len());

            file_ids.extend(
                send_videos_and_get_file_ids(&bot, &message, media_group_to_send, request_timeout)
                    .await?,
            );
        }

        media_group_file_size += input_media_size;

        media_group.push(input_media);
    }

    if media_group.is_empty() {
        event!(Level::DEBUG, "Media group is empty");

        return Ok(EventReturn::Finish);
    }

    let request_timeout = get_request_timeout_by_len(media_group.len());

    file_ids
        .extend(send_videos_and_get_file_ids(&bot, &message, media_group, request_timeout).await?);

    Ok(EventReturn::Finish)
}
