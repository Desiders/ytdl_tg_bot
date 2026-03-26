<div align="center">

<h1><code>ytdl_tg_bot</code></h1>

<h3>
A telegram bot for downloading audio and video
</h3>

</div>

Telegram: [@yv2t_bot](https://t.me/yv2t_bot)

## Installation

- Install [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/)
- Clone this repository `git clone https://github.com/Desiders/ytdl_tg_bot.git`
- Copy `.env.example` to `.env` and fill it with your data
- Copy `configs/config.example.toml` to `configs/config.toml` and fill it with your data
- Run `just docker-up` to start the project

## Optional downloader node

- Copy `configs/downloader.example.toml` to `configs/downloader.toml`
- Keep `configs/config.toml` and `configs/downloader.toml` in sync:
  `download.nodes[0].address = "http://downloader:50051"` and the token must match `auth.tokens`
- The downloader container installs yt-dlp as a Python package and launches it as `python3 -m yt_dlp`.
- If you use yt-dlp plugins, keep them under `yt-dlp/yt-dlp-plugins/` so the default downloader config can load them.
- The downloader port is published as `${DOWNLOADER_BIND_IP}:${DOWNLOADER_PORT}`.
  Set `DOWNLOADER_BIND_IP=127.0.0.1` for local-only access or `DOWNLOADER_BIND_IP=0.0.0.0` for public access.
- Run `just docker-up-downloader` to start the local downloader node too

Docker deployment files live in `deployment/`.

## Migrations

### Docker
- `just docker-migration help`

### Dev

- `cd migration`
- `cargo run -- help`

## Features

- Video download 
- Audio download
- Playlist download
- Language selection
- Skip download param
- Random media
- Media crop
- Stats
- Exclude domains list
