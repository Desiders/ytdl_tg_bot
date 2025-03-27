use either::Either::{self, Left, Right};
use std::{error::Error, future::Future, marker::PhantomData, mem, path::Path};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::{event, instrument, Level};
use url::Url;

use crate::{
    cmd::{get_media_or_playlist_info, GetMediaInfoErrorKind},
    config,
    download::{self, AudioToTempDirErrorKind, VideoErrorKind},
    handlers_utils::range::Range,
    models::{AudioInFS, Video, VideoInFS},
};

const GET_INFO_TIMEOUT: u64 = 120;
const DOWNLOAD_MEDIA_TIMEOUT: u64 = 180;
const MAX_MEDIA_GROUP_LEN: usize = 10;

pub struct DownloadMedia<'a, S> {
    url: Url,
    range: Range,
    playlist_is_allowed: bool,
    temp_dir_path: &'a Path,
    sender: S,
    yt_dlp_config: &'a config::YtDlp,
    bot_config: &'a config::Bot,
}

impl<'a, S> DownloadMedia<'a, S> {
    pub const fn new(
        url: Url,
        range: Range,
        playlist_is_allowed: bool,
        temp_dir_path: &'a Path,
        sender: S,
        yt_dlp_config: &'a config::YtDlp,
        bot_config: &'a config::Bot,
    ) -> Self {
        Self {
            url,
            range,
            playlist_is_allowed,
            temp_dir_path,
            sender,
            yt_dlp_config,
            bot_config,
        }
    }
}

pub struct DownloadInfo {
    pub is_playlist: bool,
    pub count: usize,
}

#[derive(thiserror::Error, Debug)]
pub enum DownloadErrorKind {
    #[error(transparent)]
    GetMediaInfo(#[from] GetMediaInfoErrorKind),
}

impl DownloadMedia<'_, UnboundedSender<(usize, Either<(VideoInFS, Video), VideoErrorKind>)>> {
    #[instrument(skip_all)]
    pub async fn download(self) -> Result<DownloadInfo, DownloadErrorKind> {
        let videos_info = get_media_or_playlist_info(
            &self.yt_dlp_config.full_path,
            self.url,
            self.playlist_is_allowed,
            GET_INFO_TIMEOUT,
            &self.range,
        )
        .await
        .map_err(|err| {
            event!(Level::ERROR, "{err}");
            err
        })?;
        let count = videos_info.len();

        if count == 0 {
            event!(Level::WARN, "Playlist empty");

            return Ok(DownloadInfo {
                is_playlist: true,
                count: 0,
            });
        }

        event!(Level::DEBUG, len = count, "Got videos info");

        if count == 1 {
            let video_info = videos_info.into_iter().next().unwrap();
            let max_file_size = self.yt_dlp_config.max_file_size;
            let full_path = self.yt_dlp_config.full_path.clone();
            let yt_toolkit_api_url = self.bot_config.yt_toolkit_api_url.clone();
            let temp_dir_path = self.temp_dir_path.to_owned();

            tokio::spawn(async move {
                match download::video(
                    &video_info,
                    max_file_size,
                    full_path,
                    yt_toolkit_api_url,
                    temp_dir_path,
                    DOWNLOAD_MEDIA_TIMEOUT,
                )
                .await
                {
                    Ok(video) => self.sender.send((0, Left((video, video_info)))),
                    Err(err) => self.sender.send((0, Right(err))),
                }
            });
        } else {
            for (video_index, video_info) in videos_info.enumerate() {
                let max_file_size = self.yt_dlp_config.max_file_size;
                let full_path = self.yt_dlp_config.full_path.clone();
                let yt_toolkit_api_url = self.bot_config.yt_toolkit_api_url.clone();
                let temp_dir_path = self.temp_dir_path.to_owned();
                let sender = self.sender.clone();

                tokio::spawn(async move {
                    match download::video(
                        &video_info,
                        max_file_size,
                        full_path,
                        yt_toolkit_api_url,
                        temp_dir_path,
                        DOWNLOAD_MEDIA_TIMEOUT,
                    )
                    .await
                    {
                        Ok(video) => sender.send((video_index, Left((video, video_info)))),
                        Err(err) => sender.send((video_index, Right(err))),
                    }
                });
            }
        }

        Ok(DownloadInfo { is_playlist: true, count })
    }
}

impl DownloadMedia<'_, UnboundedSender<(usize, Either<(AudioInFS, Video), AudioToTempDirErrorKind>)>> {
    #[instrument(skip_all)]
    pub async fn download(self) -> Result<DownloadInfo, DownloadErrorKind> {
        let videos_info = get_media_or_playlist_info(
            &self.yt_dlp_config.full_path,
            &self.url,
            self.playlist_is_allowed,
            GET_INFO_TIMEOUT,
            &self.range,
        )
        .await?;
        let count = videos_info.len();

        if count == 0 {
            event!(Level::WARN, "Playlist empty");

            return Ok(DownloadInfo {
                is_playlist: true,
                count: 0,
            });
        }

        event!(Level::DEBUG, len = count, "Got videos info");

        if count == 1 {
            let video_info = videos_info.into_iter().next().unwrap();
            let max_file_size = self.yt_dlp_config.max_file_size;
            let full_path = self.yt_dlp_config.full_path.clone();
            let yt_toolkit_api_url = self.bot_config.yt_toolkit_api_url.clone();
            let temp_dir_path = self.temp_dir_path.to_owned();

            tokio::spawn(async move {
                match download::audio_to_temp_dir(
                    &video_info,
                    self.url,
                    max_file_size,
                    full_path,
                    yt_toolkit_api_url,
                    temp_dir_path,
                    DOWNLOAD_MEDIA_TIMEOUT,
                )
                .await
                {
                    Ok(audio) => self.sender.send((0, Left((audio, video_info)))),
                    Err(err) => self.sender.send((0, Right(err))),
                }
            });
        } else {
            for (video_index, video_info) in videos_info.enumerate() {
                let max_file_size = self.yt_dlp_config.max_file_size;
                let full_path = self.yt_dlp_config.full_path.clone();
                let yt_toolkit_api_url = self.bot_config.yt_toolkit_api_url.clone();
                let temp_dir_path = self.temp_dir_path.to_owned();
                let sender = self.sender.clone();

                tokio::spawn(async move {
                    let video_id = video_info.id.clone();

                    match download::audio_to_temp_dir(
                        &video_info,
                        video_id,
                        max_file_size,
                        full_path,
                        yt_toolkit_api_url,
                        temp_dir_path,
                        DOWNLOAD_MEDIA_TIMEOUT,
                    )
                    .await
                    {
                        Ok(audio) => sender.send((video_index, Left((audio, video_info)))),
                        Err(err) => sender.send((video_index, Right(err))),
                    }
                });
            }
        }

        Ok(DownloadInfo { is_playlist: true, count })
    }
}

pub struct SendMedia<T, SingleF, SingleFut, SingleErr, GroupF, GroupFut, GroupErr, R, RecvErr> {
    is_playlist: bool,
    single_sender: SingleF,
    group_sender: GroupF,
    receiver: R,
    _phantom: PhantomData<(T, SingleFut, SingleErr, RecvErr, GroupFut, GroupErr)>,
}

impl<T, SingleF, SingleFut, SingleErr, RecvErr, GroupF, GroupFut, GroupErr>
    SendMedia<T, SingleF, SingleFut, SingleErr, GroupF, GroupFut, GroupErr, UnboundedReceiver<(usize, Either<T, RecvErr>)>, RecvErr>
{
    pub const fn new(
        is_playlist: bool,
        single_sender: SingleF,
        group_sender: GroupF,
        receiver: UnboundedReceiver<(usize, Either<T, RecvErr>)>,
    ) -> Self
    where
        SingleF: FnMut(T) -> SingleFut,
        SingleFut: Future<Output = Result<MediaId, SingleErr>>,
        GroupF: FnMut(Vec<MediaId>) -> GroupFut,
        GroupFut: Future<Output = Result<(), GroupErr>>,
    {
        Self {
            is_playlist,
            single_sender,
            group_sender,
            receiver,
            _phantom: PhantomData,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SendVideosErrorKind<GroupErr> {
    #[error(transparent)]
    SendGroup(#[from] GroupErr),
}

type MediaId = String;

impl<T, SingleF, SingleFut, SingleErr, RecvErr, GroupF, GroupFut, GroupErr>
    SendMedia<T, SingleF, SingleFut, SingleErr, GroupF, GroupFut, GroupErr, UnboundedReceiver<(usize, Either<T, RecvErr>)>, RecvErr>
where
    SingleF: FnMut(T) -> SingleFut,
    SingleFut: Future<Output = Result<MediaId, SingleErr>>,
    SingleErr: Error + Send + 'static,
    GroupF: FnMut(Vec<MediaId>) -> GroupFut,
    GroupFut: Future<Output = Result<(), GroupErr>>,
    GroupErr: Error,
    RecvErr: Error + Send + 'static,
{
    #[instrument(skip_all)]
    pub async fn send(mut self) -> Result<Vec<(usize, Box<dyn Error + Send>)>, SendVideosErrorKind<GroupErr>> {
        let mut failed_sends: Vec<(usize, Box<dyn Error + Send>)> = vec![];
        let mut media_group = Vec::with_capacity(MAX_MEDIA_GROUP_LEN);

        loop {
            match self.receiver.recv().await {
                Some((index, Left(media))) => match (self.single_sender)(media).await {
                    Ok(media_id) => media_group.push(media_id),
                    Err(err) => {
                        failed_sends.push((index, Box::new(err)));
                    }
                },
                Some((index, Right(err))) => {
                    failed_sends.push((index, Box::new(err)));
                }
                None => break,
            };

            if media_group.len() == MAX_MEDIA_GROUP_LEN {
                (self.group_sender)(mem::take(&mut media_group)).await?;
            }
        }

        if media_group.len() != 0 {
            (self.group_sender)(media_group).await?;
        }

        Ok(failed_sends)
    }
}
