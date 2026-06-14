use std::{
    io,
    path::Path,
    process::{Output, Stdio},
    sync::Arc,
    time::Duration,
};

use serde::Deserialize;
use tempfile::TempDir;
use tokio::time;
use tracing::{info, instrument, warn};

use crate::{config::SongrecConfig, utils::process_exit_error};

const RECOGNIZE_TIMEOUT_SECS: u64 = 45;
const FFMPEG_PATH: &str = "/usr/bin/ffmpeg";

#[derive(Debug, thiserror::Error)]
pub enum RecognizeErrorKind {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Empty audio")]
    EmptyAudio,
    #[error("Could not decode audio")]
    Decode,
    #[error("Could not recognize the song")]
    NoMatch,
    #[error("Failed to parse SongRec output: {0}")]
    Parse(String),
    #[error("SongRec timed out")]
    Timeout,
    #[error("Song recognizer is disabled")]
    Disabled,
}

/// A recognized song's metadata, from Shazam via SongRec.
#[derive(Debug, Clone)]
pub struct Recognized {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub url: Option<String>,
    pub cover_url: Option<String>,
}

pub struct SongRecognizer {
    cfg: Arc<SongrecConfig>,
}

impl SongRecognizer {
    #[must_use]
    pub const fn new(cfg: Arc<SongrecConfig>) -> Self {
        Self { cfg }
    }

    /// Recognizes a song from a short audio clip.
    ///
    /// The clip is first transcoded to mono WAV with the ffmpeg binary (so any container/codec a
    /// user can send — Telegram voice is Opus — is accepted), then handed to `songrec`, which is
    /// built without its ffmpeg feature and only needs to read WAV.
    #[instrument(skip_all)]
    pub async fn recognize(&self, audio: &[u8]) -> Result<Recognized, RecognizeErrorKind> {
        if !self.cfg.enabled {
            return Err(RecognizeErrorKind::Disabled);
        }
        if audio.is_empty() {
            return Err(RecognizeErrorKind::EmptyAudio);
        }

        let temp_dir = TempDir::with_prefix("ytdl-tg-bot-songrec-")?;
        let input_path = temp_dir.path().join("input");
        let wav_path = temp_dir.path().join("audio.wav");
        tokio::fs::write(&input_path, audio).await?;

        info!(bytes = audio.len(), "Recognizing song");
        let started_at = time::Instant::now();

        self.transcode_to_wav(&input_path, &wav_path).await?;
        let recognized = self.run_songrec(&wav_path).await?;

        info!(
            elapsed_ms = started_at.elapsed().as_millis(),
            title = recognized.title.as_deref(),
            artist = recognized.artist.as_deref(),
            "Recognized song"
        );
        Ok(recognized)
    }

    async fn transcode_to_wav(&self, input: &Path, wav: &Path) -> Result<(), RecognizeErrorKind> {
        let child = tokio::process::Command::new(FFMPEG_PATH)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(input)
            .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "wav"])
            .arg(wav)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let Output { status, stderr, .. } = time::timeout(Duration::from_secs(RECOGNIZE_TIMEOUT_SECS), child.wait_with_output())
            .await
            .map_err(|_| RecognizeErrorKind::Timeout)??;

        if !status.success() {
            warn!(stderr = %String::from_utf8_lossy(&stderr), "ffmpeg transcode for recognition failed");
            return Err(RecognizeErrorKind::Decode);
        }
        Ok(())
    }

    async fn run_songrec(&self, wav: &Path) -> Result<Recognized, RecognizeErrorKind> {
        let (program, base_args) = self.cfg.command_parts();
        let child = tokio::process::Command::new(program)
            .args(base_args)
            .arg("audio-file-to-recognized-song")
            .arg(wav)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let Output { status, stdout, stderr } = time::timeout(Duration::from_secs(RECOGNIZE_TIMEOUT_SECS), child.wait_with_output())
            .await
            .map_err(|_| RecognizeErrorKind::Timeout)??;

        let stderr = String::from_utf8_lossy(&stderr);
        if !status.success() {
            return Err(process_exit_error("SongRec", status, stderr.trim()).into());
        }
        if !stderr.is_empty() {
            warn!(%stderr);
        }

        parse_recognized(&String::from_utf8_lossy(&stdout))
    }
}

/// `songrec audio-file-to-recognized-song` prints the raw Shazam response. A successful match has a
/// `track` object; a no-match response omits it (only `matches: []` / `retryms`).
fn parse_recognized(stdout: &str) -> Result<Recognized, RecognizeErrorKind> {
    let response: ShazamResponse = serde_json::from_str(stdout.trim()).map_err(|err| RecognizeErrorKind::Parse(err.to_string()))?;
    let Some(track) = response.track else {
        return Err(RecognizeErrorKind::NoMatch);
    };

    let cover_url = track.images.and_then(|images| images.coverarthq.or(images.coverart));
    let album = track
        .sections
        .into_iter()
        .flat_map(|section| section.metadata)
        .find(|meta| meta.title.as_deref() == Some("Album"))
        .and_then(|meta| meta.text);

    Ok(Recognized {
        title: track.title,
        artist: track.subtitle,
        album,
        url: track.url,
        cover_url,
    })
}

#[derive(Debug, Deserialize)]
struct ShazamResponse {
    #[serde(default)]
    track: Option<ShazamTrack>,
}

#[derive(Debug, Deserialize)]
struct ShazamTrack {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    subtitle: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    images: Option<ShazamImages>,
    #[serde(default)]
    sections: Vec<ShazamSection>,
}

#[derive(Debug, Deserialize)]
struct ShazamImages {
    #[serde(default)]
    coverart: Option<String>,
    #[serde(default)]
    coverarthq: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShazamSection {
    #[serde(default)]
    metadata: Vec<ShazamMeta>,
}

#[derive(Debug, Deserialize)]
struct ShazamMeta {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{parse_recognized, RecognizeErrorKind};

    #[test]
    fn parses_matched_track() {
        let json = r#"{
            "track": {
                "title": "Bohemian Rhapsody",
                "subtitle": "Queen",
                "url": "https://www.shazam.com/track/5933914",
                "images": { "coverart": "https://img/lo.jpg", "coverarthq": "https://img/hq.jpg" },
                "sections": [
                    { "type": "SONG", "metadata": [ { "title": "Album", "text": "A Night at the Opera" } ] }
                ]
            }
        }"#;
        let r = parse_recognized(json).unwrap();
        assert_eq!(r.title.as_deref(), Some("Bohemian Rhapsody"));
        assert_eq!(r.artist.as_deref(), Some("Queen"));
        assert_eq!(r.album.as_deref(), Some("A Night at the Opera"));
        assert_eq!(r.url.as_deref(), Some("https://www.shazam.com/track/5933914"));
        assert_eq!(r.cover_url.as_deref(), Some("https://img/hq.jpg"));
    }

    #[test]
    fn no_track_is_no_match() {
        assert!(matches!(
            parse_recognized(r#"{"matches": [], "retryms": 2000}"#),
            Err(RecognizeErrorKind::NoMatch)
        ));
    }

    #[test]
    fn missing_optionals_are_none() {
        let r = parse_recognized(r#"{"track": {"title": "X"}}"#).unwrap();
        assert_eq!(r.title.as_deref(), Some("X"));
        assert!(r.artist.is_none() && r.album.is_none() && r.cover_url.is_none());
    }
}
