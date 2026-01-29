use crate::entities::{language::Language, Cookie, Playlist, Range};

use serde::de::DeserializeOwned;
use std::{
    io::{self, BufRead as _},
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{io::AsyncBufReadExt as _, sync::mpsc};
use tracing::{error, instrument, trace, warn};

pub enum FormatStrategy<'a> {
    VideoAndAudio,
    AudioOnly { audio_ext: &'a str },
}

impl FormatStrategy<'_> {
    fn templates(&self) -> &[&str] {
        match self {
            Self::VideoAndAudio => &["bv{video_args}+ba{audio_args}/b{combined_args}"],
            Self::AudioOnly { .. } => &["ba{audio_args}"],
        }
    }

    fn fallbacks(&self) -> &[&str] {
        match self {
            Self::VideoAndAudio => &["bv*+ba", "b", "w"],
            Self::AudioOnly { .. } => &["ba", "wa", "ba*"],
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseJsonErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum GetInfoErrorKind {
    #[error(transparent)]
    Ndjson(#[from] ParseJsonErrorKind),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadErrorKind {
    #[error(transparent)]
    Ndjson(#[from] ParseJsonErrorKind),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

fn parse_ndjson<T: DeserializeOwned>(input: &[u8]) -> Result<Vec<T>, ParseJsonErrorKind> {
    use std::io::BufReader;

    let lines = BufReader::new(input).lines();
    let mut results = Vec::with_capacity(1);
    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let item = serde_json::from_str(&line)?;
        results.push(item);
    }
    Ok(results)
}

pub fn build_formats_string(strategy: FormatStrategy, heights: &[u32], audio_language: &Language) -> String {
    fn push_if_some<T>(vec: &mut Vec<T>, value: Option<T>) {
        if let Some(v) = value {
            vec.push(v);
        }
    }

    fn render_args(args: &[String]) -> String {
        args.iter().map(|val| format!("[{val}]")).collect()
    }

    let templates = strategy.templates();
    let fallbacks = strategy.fallbacks();

    let mut formats = Vec::with_capacity(heights.len() * templates.len() + fallbacks.len());

    match strategy {
        FormatStrategy::AudioOnly { .. } => {
            let mut audio_args = vec![];
            push_if_some(
                &mut audio_args,
                audio_language.language.as_deref().map(|lang| format!("language^={lang}")),
            );

            for &template in templates {
                let format = template.replace("{audio_args}", &render_args(&audio_args));
                formats.push(format);
            }

            for &fallback in fallbacks {
                formats.push(fallback.to_owned());
            }
        }

        FormatStrategy::VideoAndAudio => {
            for &height in heights {
                let mut video_args = vec!["vcodec!=none".to_owned()];
                let mut audio_args = vec![];
                let mut combined_args = vec![];

                push_if_some(&mut video_args, Some(format!("height<={height}")));
                push_if_some(&mut combined_args, Some(format!("height<={height}")));
                push_if_some(
                    &mut audio_args,
                    audio_language.language.as_deref().map(|lang| format!("language^={lang}")),
                );

                for &template in templates {
                    let format = template
                        .replace("{video_args}", &render_args(&video_args))
                        .replace("{audio_args}", &render_args(&audio_args))
                        .replace("{combined_args}", &render_args(&combined_args));

                    formats.push(format);
                }
            }

            for &fallback in fallbacks {
                formats.push(fallback.to_owned());
            }
        }
    }

    formats.join(",")
}

#[instrument(skip_all)]
pub async fn get_media_info(
    search: &str,
    strategy: FormatStrategy<'_>,
    audio_language: &Language,
    executable_path: &str,
    pot_provider_url: &str,
    playlist_range: &Range,
    allow_playlist: bool,
    timeout: u64,
    cookie: Option<&Cookie>,
) -> Result<Playlist, GetInfoErrorKind> {
    use tokio::{process::Command, time};

    let playlist_range = playlist_range.to_range_string();
    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={pot_provider_url}");

    let heights = [2160, 1440, 1080, 720, 480, 360, 240, 144];
    let formats = build_formats_string(strategy, &heights, audio_language);

    let mut args = vec![
        "--js-runtimes",
        "deno:deno",
        "--print",
        "%()j",
        "--socket-timeout",
        "5",
        "--no-write-comments",
        "--no-download",
        "--no-progress",
        "--no-config",
        "--no-color",
        "-I",
        &playlist_range,
        "--format-sort",
        "ext,quality,codec,source,lang",
        "-f",
        &formats,
    ];

    if allow_playlist {
        args.push("--yes-playlist");
    } else {
        args.push("--no-playlist");
    }

    args.push("--extractor-args");
    args.push(&extractor_arg);
    args.push("--extractor-args");
    args.push("youtube:player_client=default,mweb,web_music,web_creator;player_skip=configs,initial_data;use_ad_playback_context=true");

    let cookie_path = cookie.map(|val| val.path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        trace!("Using cookies from: {cookie_path}");
        args.push("--cookies");
        args.push(cookie_path);
    } else {
        trace!("No cookies provided");
    }

    args.push("--");
    args.push(search);

    trace!("{args:?}");

    let child = Command::new(executable_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
        Ok(Ok(Output { status, stdout, stderr })) => {
            let stderr = String::from_utf8_lossy(&stderr);
            if status.success() {
                if !stderr.is_empty() {
                    warn!(%stderr);
                }
                match parse_ndjson(&stdout) {
                    Ok(val) => Ok(Playlist::new(val)),
                    Err(err) => Err(err.into()),
                }
            } else {
                match status.code() {
                    Some(code) => Err(io::Error::other(format!("Ytdlp exited with code {code} and message: {stderr}")).into()),
                    None => Err(io::Error::other(format!("Ytdlp exited with and message: {stderr}")).into()),
                }
            }
        }
        Ok(Err(err)) => Err(err.into()),
        Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Ytdlp timed out").into()),
    }
}

#[instrument(skip_all)]
pub async fn download_media(
    search: &str,
    strategy: FormatStrategy<'_>,
    format_id: &str,
    max_filesize: u64,
    output_dir_path: &Path,
    executable_path: &str,
    pot_provider_url: &str,
    timeout: u64,
    cookie: Option<&Cookie>,
    progress_sender: Option<mpsc::UnboundedSender<String>>,
) -> Result<(), DownloadErrorKind> {
    use tokio::{io::BufReader, process::Command, time};

    let output_dir_path = output_dir_path.to_string_lossy();
    let max_filesize = max_filesize.to_string();
    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={pot_provider_url}");

    let mut args = vec![
        "--js-runtimes",
        "deno:deno",
        "--socket-timeout",
        "5",
        "-R",
        "3",
        "--file-access-retries",
        "2",
        "--fragment-retries",
        "3",
        "--concurrent-fragments",
        "4",
        "--newline",
        "--progress-template",
        "download-progress:%(progress._default_template)s",
        "--progress-delta",
        "3",
        "--max-filesize",
        &max_filesize,
        "--add-metadata",
        "--no-write-comments",
        "--no-playlist",
        "--no-config",
        "--no-color",
        "--paths",
        &output_dir_path,
        "--output",
        "%(id)s.%(ext)s",
        "-f",
        &format_id,
    ];

    match strategy {
        FormatStrategy::VideoAndAudio => {}
        FormatStrategy::AudioOnly { audio_ext } => {
            args.push("--embed-thumbnail");
            args.push("--write-thumbnail");
            args.push("--extract-audio");
            args.extend(["--audio-format", audio_ext]);
        }
    };

    args.push("--extractor-args");
    args.push(&extractor_arg);
    args.push("--extractor-args");
    args.push("youtube:player_client=default,mweb,web_music,web_creator;player_skip=configs,initial_data;use_ad_playback_context=true");

    let cookie_path = cookie.map(|val| val.path.to_string_lossy());
    if let Some(cookie_path) = cookie_path.as_deref() {
        trace!("Using cookies from: {cookie_path}");
        args.push("--cookies");
        args.push(cookie_path);
    } else {
        trace!("No cookies provided");
    }

    args.push("--");
    args.push(search);

    let mut child = Command::new(executable_path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout).lines();

    let ((), res) = tokio::join!(
        async {
            let Some(progress_sender) = progress_sender else {
                return;
            };
            while let Ok(Some(line)) = reader.next_line().await {
                if !line.starts_with("download-progress") {
                    continue;
                }
                let Some((_, progress)) = line.split_once(':') else {
                    continue;
                };
                if let Err(err) = progress_sender.send(progress.to_owned()) {
                    error!(%err, "Send progress err");
                    return;
                }
            }
        },
        async {
            match time::timeout(Duration::from_secs(timeout), child.wait_with_output()).await {
                Ok(Ok(Output { status, stderr, .. })) => {
                    let stderr = String::from_utf8_lossy(&stderr);
                    if status.success() {
                        if !stderr.is_empty() {
                            warn!(%stderr);
                        }
                        Ok(())
                    } else {
                        match status.code() {
                            Some(code) => Err(io::Error::other(format!("Ytdlp exited with code {code} and message: {stderr}")).into()),
                            None => Err(io::Error::other(format!("Ytdlp exited with and message: {stderr}")).into()),
                        }
                    }
                }
                Ok(Err(err)) => Err(err.into()),
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "Ytdlp timed out").into()),
            }
        }
    );
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_and_audio_builds_formats_for_each_height() {
        let heights = [1080, 720];

        let result = build_formats_string(FormatStrategy::VideoAndAudio, &heights, &Language::default());

        assert_eq!(
            result,
            "bv[vcodec!=none,height<=1080]+ba[]/b[height<=1080],bv[vcodec!=none,height<=720]+ba[]/b[height<=720],bv*+ba,b,w"
        );
    }

    #[test]
    fn video_and_audio_audio_language_is_added_when_provided() {
        let heights = [1080];

        let result = build_formats_string(
            FormatStrategy::VideoAndAudio,
            &heights,
            &Language {
                language: Some("ru".to_owned()),
            },
        );

        assert_eq!(result, "bv[vcodec!=none,height<=1080]+ba[language^=ru]/b[height<=1080],bv*+ba,b,w");
    }

    #[test]
    fn video_and_audio_audio_language_is_not_added_when_none() {
        let heights = [1080];

        let result = build_formats_string(FormatStrategy::VideoAndAudio, &heights, &Language::default());

        assert_eq!(result, "bv[vcodec!=none,height<=1080]+ba[]/b[height<=1080],bv*+ba,b,w");
    }

    #[test]
    fn audio_only_builds_formats_correctly_without_heights() {
        let result = build_formats_string(FormatStrategy::AudioOnly { audio_ext: "m4a" }, &[], &Language::default());

        assert_eq!(result, "ba[],ba,wa");
    }

    #[test]
    fn audio_only_includes_language_if_provided() {
        let result = build_formats_string(
            FormatStrategy::AudioOnly { audio_ext: "m4a" },
            &[],
            &Language {
                language: Some("en".to_owned()),
            },
        );

        assert_eq!(result, "ba[language^=en],ba,wa");
    }

    #[test]
    fn audio_only_multiple_languages_are_handled_correctly() {
        let result = build_formats_string(
            FormatStrategy::AudioOnly { audio_ext: "m4a" },
            &[],
            &Language {
                language: Some("ru".to_owned()),
            },
        );

        assert_eq!(result, "ba[language^=ru],ba,wa");
    }

    #[test]
    fn video_and_audio_multiple_heights_preserve_order() {
        let heights = [2160, 1080, 720];

        let result = build_formats_string(FormatStrategy::VideoAndAudio, &heights, &Language::default());

        assert_eq!(
            result,
            "bv[vcodec!=none,height<=2160]+ba[]/b[height<=2160],bv[vcodec!=none,height<=1080]+ba[]/b[height<=1080],bv[vcodec!=none,height<=720]+ba[]/b[height<=720],bv*+ba,b,w"
        );
    }
}
