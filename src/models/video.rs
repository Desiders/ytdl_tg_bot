use serde::{Deserialize, Serialize};
use std::ops::Deref;
use youtube_dl::{Format as YtdlFormat, SingleVideo, Thumbnail, YoutubeDlOutput};

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Format {
    pub format_id: Option<String>,
    pub format: Option<String>,
    pub format_note: Option<String>,
    pub ext: Option<String>,
    pub resolution: Option<String>,
    pub url: Option<String>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl From<YtdlFormat> for Format {
    fn from(format: YtdlFormat) -> Self {
        Self {
            format_id: format.format_id,
            format: format.format,
            format_note: format.format_note,
            ext: format.ext,
            resolution: format.resolution,
            url: format.url,
            filesize: format.filesize,
            filesize_approx: format.filesize_approx,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Video {
    pub id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub thumbnail: Option<String>,
    pub thumbnails: Option<Vec<Thumbnail>>,
    pub formats: Option<Vec<Format>>,
}

impl Video {
    pub fn get_best_thumbnail(&self) -> Option<&Thumbnail> {
        let Some(thumbnails) = self.thumbnails.as_ref() else {
            return None;
        };

        thumbnails
            .iter()
            .filter(|thumbnail| thumbnail.url.is_some())
            .max_by_key(|thumbnail| thumbnail.filesize)
    }
}

impl From<SingleVideo> for Video {
    fn from(video: SingleVideo) -> Self {
        Self {
            id: video.id,
            title: video.title,
            url: video.url,
            thumbnail: video.thumbnail,
            thumbnails: video.thumbnails,
            formats: video
                .formats
                .map(|formats| formats.into_iter().map(Format::from).collect()),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Videos(pub Vec<Video>);

impl Videos {
    pub fn get_by_id(&self, id: &str) -> Option<&Video> {
        self.0.iter().find(|video| video.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Iterator for Videos {
    type Item = Video;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop()
    }
}

impl Extend<Video> for Videos {
    fn extend<T: IntoIterator<Item = Video>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl Deref for Videos {
    type Target = Vec<Video>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<YoutubeDlOutput> for Videos {
    fn from(ytdl_output: YoutubeDlOutput) -> Self {
        match ytdl_output {
            YoutubeDlOutput::SingleVideo(video) => Self(vec![Video::from(*video)]),
            YoutubeDlOutput::Playlist(playlist) => {
                let Some(entries) = playlist.entries else {
                    return Self::default();
                };

                Self(entries.into_iter().map(Video::from).collect())
            }
        }
    }
}
