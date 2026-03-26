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

## Optional downloader TLS

- The simplest rollout is server-only TLS plus the existing bearer token auth.
- Generate one private CA on the bot/admin machine, then issue one server cert per downloader node.
- Use SANs that exactly match what the bot will dial.

Example commands:

```bash
chmod +x deployment/generate-node-certs.sh

# Local node on the same host as the bot.
deployment/generate-node-certs.sh \
  --root-dir ./tls \
  --node local \
  --ip 127.0.0.1 \
  --dns localhost

# External node published on a fixed server IP.
deployment/generate-node-certs.sh \
  --root-dir ./tls \
  --node external \
  --ip 203.0.113.10
```

- Keep `tls/ca/ca.key` only on the machine that issues certificates.
- Give each downloader node its own `server.crt` and `server.key` from `tls/nodes/<name>/`.
- Give the bot only `ca.crt` so it can verify node certificates.
- If the bot connects to `https://127.0.0.1:50051`, the cert must contain `IP:127.0.0.1`.
- If the bot connects to `https://203.0.113.10:50051`, the cert must contain `IP:203.0.113.10`.
- Configure the bot with `[download.tls].ca_cert_path` and switch only TLS-backed nodes to `https://...`.
- Configure each TLS-enabled downloader node with `[tls].cert_path` and `[tls].key_path`.

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
