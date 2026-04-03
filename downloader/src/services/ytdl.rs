use crate::{
    config::YtDlpConfig,
    entities::{language::Language, Cookie, Playlist, Range, Sections},
};

use serde::de::DeserializeOwned;
use std::{
    fmt::Write as _,
    io::{self, BufRead as _},
    path::Path,
    process::{Output, Stdio},
    time::Duration,
};
use tokio::{io::AsyncBufReadExt as _, sync::mpsc};
use tracing::{debug, error, instrument, trace, warn};

#[derive(Debug, Clone)]
pub enum FormatStrategy {
    VideoAndAudio,
    AudioOnly { audio_ext: String },
}

impl FormatStrategy {
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
    #[error("Yt-dlp requires a different node context: {0}")]
    Retryable(RetryableYtdlpError),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadErrorKind {
    #[error(transparent)]
    Ndjson(#[from] ParseJsonErrorKind),
    #[error("Yt-dlp requires a different node context: {0}")]
    Retryable(RetryableYtdlpError),
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum RetryableYtdlpError {
    #[error("Authentication required")]
    AuthenticationRequired,
    #[error("Geo restricted")]
    GeoRestricted,
}

fn classify_retryable_error(stderr: &str) -> Option<RetryableYtdlpError> {
    let stderr = stderr.to_ascii_lowercase();

    let auth_required = [
        "only available for registered users",
        "login required",
        "sign in",
        "use --cookies",
        "use --cookies-from-browser",
        "use --username and --password",
        "authentication required",
    ]
    .iter()
    .any(|pattern| stderr.contains(pattern));
    if auth_required {
        return Some(RetryableYtdlpError::AuthenticationRequired);
    }

    let geo_restricted = [
        "geo restricted",
        "geo restriction",
        "not available from your location",
        "not available in your country",
        "this content is not available in your location",
    ]
    .iter()
    .any(|pattern| stderr.contains(pattern));
    if geo_restricted {
        return Some(RetryableYtdlpError::GeoRestricted);
    }

    None
}

fn parse_ndjson<T: DeserializeOwned>(input: &[u8]) -> Result<Vec<(T, String)>, ParseJsonErrorKind> {
    use std::io::BufReader;

    let lines = BufReader::new(input).lines();
    let mut results = Vec::with_capacity(1);
    for line in lines {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let item = serde_json::from_str(&line)?;
        results.push((item, line));
    }
    Ok(results)
}

pub fn build_formats_string(strategy: &FormatStrategy, heights: &[u32], audio_language: &Language) -> String {
    fn push_if_some<T>(vec: &mut Vec<T>, value: Option<T>) {
        if let Some(v) = value {
            vec.push(v);
        }
    }

    fn render_args(args: &[String]) -> String {
        let mut rendered = String::new();
        for arg in args {
            let _ = write!(rendered, "[{arg}]");
        }
        rendered
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
                let mut video_args = vec!["vcodec!=none".to_owned(), "vcodec!*=av01".to_owned()];
                let mut audio_args = vec![];
                let mut combined_args = vec!["vcodec!*=av01".to_owned()];

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
#[allow(clippy::too_many_arguments)]
pub async fn get_media_info(
    search: &str,
    strategy: &FormatStrategy,
    audio_language: &Language,
    yt_dlp_cfg: &YtDlpConfig,
    pot_provider_url: &str,
    playlist_range: &Range,
    allow_playlist: bool,
    timeout: u64,
    cookie: Option<&Cookie>,
) -> Result<Playlist, GetInfoErrorKind> {
    use tokio::time;

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
        "--compat-options",
        "manifest-filesize-approx",
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

    trace!(?args, "Ytdlp args");

    let child = create_ytdlp_command(yt_dlp_cfg)
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
                if let Some(kind) = classify_retryable_error(&stderr) {
                    return Err(GetInfoErrorKind::Retryable(kind));
                }
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
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn download_media(
    strategy: FormatStrategy,
    format_id: &str,
    sections: Option<&Sections>,
    max_filesize: u64,
    output_dir_path: &Path,
    info_file_path: &Path,
    yt_dlp_cfg: &YtDlpConfig,
    pot_provider_url: &str,
    timeout: u64,
    cookie: Option<&Cookie>,
    progress_sender: Option<&mpsc::UnboundedSender<String>>,
) -> Result<(), DownloadErrorKind> {
    use tokio::{io::BufReader, time};

    let max_filesize = max_filesize.to_string();
    let output_dir_path = output_dir_path.to_string_lossy();
    let info_file_path = info_file_path.to_string_lossy();
    let extractor_arg = format!("youtubepot-bgutilhttp:base_url={pot_provider_url}");
    let audio_ext = match &strategy {
        FormatStrategy::VideoAndAudio => None,
        FormatStrategy::AudioOnly { audio_ext } => Some(audio_ext.as_str()),
    };

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
        "--no-write-comments",
        "--no-playlist",
        "--no-config",
        "--no-color",
        "--paths",
        &output_dir_path,
        "--output",
        "media.%(ext)s",
        "--no-windows-filenames",
        "--load-info-json",
        &info_file_path,
        "--add-metadata",
        "-f",
        &format_id,
    ];

    if let Some(audio_ext) = audio_ext {
        args.push("--extract-audio");
        args.extend(["--audio-format", audio_ext]);
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

    let sections = sections.map(Sections::to_download_sections_string);
    if let Some(sections) = sections.as_deref() {
        args.push("--download-sections");
        args.push(sections);
    }

    trace!(?args, "Ytdlp args");

    let mut child = create_ytdlp_command(yt_dlp_cfg)
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
                    debug!("{line}");
                    continue;
                }
                let Some((_, progress)) = line.split_once(':') else {
                    continue;
                };
                if let Err(err) = progress_sender.send(progress.to_owned()) {
                    error!(%err, "Send progress error");
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
                            warn!("{stderr}");
                        }
                        Ok(())
                    } else {
                        if let Some(kind) = classify_retryable_error(&stderr) {
                            return Err(DownloadErrorKind::Retryable(kind));
                        }
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

fn create_ytdlp_command(yt_dlp_cfg: &YtDlpConfig) -> tokio::process::Command {
    let (program, base_args) = yt_dlp_cfg.command_parts();
    let mut command = tokio::process::Command::new(program);
    command.args(base_args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_and_audio_builds_formats_for_each_height() {
        let heights = [1080, 720];

        let result = build_formats_string(&FormatStrategy::VideoAndAudio, &heights, &Language::default());

        assert_eq!(
            result,
            "bv[vcodec!=none][vcodec!*=av01][height<=1080]+ba/b[vcodec!*=av01][height<=1080],\
            bv[vcodec!=none][vcodec!*=av01][height<=720]+ba/b[vcodec!*=av01][height<=720],\
            bv*+ba,b,w"
        );
    }

    #[test]
    fn video_and_audio_audio_language_is_added_when_provided() {
        let heights = [1080];

        let result = build_formats_string(
            &FormatStrategy::VideoAndAudio,
            &heights,
            &Language {
                language: Some("ru".to_owned()),
            },
        );

        assert_eq!(
            result,
            "bv[vcodec!=none][vcodec!*=av01][height<=1080]+ba[language^=ru]/b[vcodec!*=av01][height<=1080],bv*+ba,b,w"
        );
    }

    #[test]
    fn video_and_audio_audio_language_is_not_added_when_none() {
        let heights = [1080];

        let result = build_formats_string(&FormatStrategy::VideoAndAudio, &heights, &Language::default());

        assert_eq!(
            result,
            "bv[vcodec!=none][vcodec!*=av01][height<=1080]+ba/b[vcodec!*=av01][height<=1080],bv*+ba,b,w"
        );
    }

    #[test]
    fn audio_only_builds_formats_correctly_without_heights() {
        let result = build_formats_string(
            &FormatStrategy::AudioOnly {
                audio_ext: "m4a".to_owned(),
            },
            &[],
            &Language::default(),
        );

        assert_eq!(result, "ba,ba,wa,ba*");
    }

    #[test]
    fn audio_only_includes_language_if_provided() {
        let result = build_formats_string(
            &FormatStrategy::AudioOnly {
                audio_ext: "m4a".to_owned(),
            },
            &[],
            &Language {
                language: Some("en".to_owned()),
            },
        );

        assert_eq!(result, "ba[language^=en],ba,wa,ba*");
    }

    #[test]
    fn audio_only_multiple_languages_are_handled_correctly() {
        let result = build_formats_string(
            &FormatStrategy::AudioOnly {
                audio_ext: "m4a".to_owned(),
            },
            &[],
            &Language {
                language: Some("ru".to_owned()),
            },
        );

        assert_eq!(result, "ba[language^=ru],ba,wa,ba*");
    }

    #[test]
    fn video_and_audio_multiple_heights_preserve_order() {
        let heights = [2160, 1080, 720];

        let result = build_formats_string(&FormatStrategy::VideoAndAudio, &heights, &Language::default());

        assert_eq!(
            result,
            "bv[vcodec!=none][vcodec!*=av01][height<=2160]+ba/b[vcodec!*=av01][height<=2160],bv[vcodec!=none]\
            [vcodec!*=av01][height<=1080]+ba/b[vcodec!*=av01][height<=1080],bv[vcodec!=none][vcodec!*=av01]\
            [height<=720]+ba/b[vcodec!*=av01][height<=720],bv*+ba,b,w"
        );
    }

    #[test]
    fn detects_retryable_authentication_error() {
        assert_eq!(
            classify_retryable_error(
                "ERROR: [site] This video is only available for registered users. Use --cookies-from-browser or --cookies for the authentication"
            ),
            Some(RetryableYtdlpError::AuthenticationRequired)
        );
    }

    #[test]
    fn detects_retryable_geo_restricted_error() {
        assert_eq!(
            classify_retryable_error("ERROR: [site] This video is not available from your location due to geo restriction"),
            Some(RetryableYtdlpError::GeoRestricted)
        );
    }

    #[test]
    fn ignores_non_retryable_no_formats_error() {
        assert_eq!(classify_retryable_error("ERROR: [site] No video formats found!"), None);
    }
}
