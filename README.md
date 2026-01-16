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
- Copy `config.toml.example` to `config.toml` and fill it with your data
- Run `just docker-up` to start the project

## Features

### 1. Video download
- **Private chat**:  
  - Send a direct video link, and the bot will reply with the video file;  
  - Reply to a message containing a video link with any text;  
  - Use the command `/vd <url>` (`/video_download <url>`), or reply with the command without specifying the URL;  
  - Use inline mode by typing `@bot_username <url>` or `@bot_username <text>` to search for a video by its name.  

- **Group chats**:  
  - Send a direct video link, and the bot will reply with the video file;  
  - Use the command `/vd <url>` (`/video_download <url>`), or reply with the command without specifying the URL;
  - Use inline mode by typing `@bot_username <url>` or `@bot_username <text>` to search for a video by its name.  

### 2. Audio download
- **Private chat** and **group chats**:  
  - Use the command `/ad <url>` (`/audio_download <url>`), or reply with the command without specifying the URL;  
  - Use inline mode by typing `@bot_username <url>` or `@bot_username <text>` to search for an audio by its name. 

### 3. Playlist download
- Supports downloading playlists with range selection.\
  Format: `[/command] <url> [items=start:count:step]`.\
  Example:
    - `https://youtube.com/playlist?list=xxxx [items=1:10:1]`;
    - `https://youtube.com/playlist?list=xxxx [items=1:10:]`;
    - `https://youtube.com/playlist?list=xxxx [items=:10:]`;
    - `https://youtube.com/playlist?list=xxxx [items=::]`.

### 4. Language selection
- Supports language specification for subtitles or localized metadata.\
  Format: `[/command] <url> [lang=ru|en|en-US|en-GB]`.

### 5. File size and quality
- The bot downloads media in the best available quality with a maximum file size limit of **500 MB** (by default, but can be changed in the config).

### 6. Skip download param
- If in query parameters specified any of keys below with value `false`, download will be skipped.\
  Keys: `yv2t`, `yv2t_bot`, `download`.\
  Example:
    - `https://youtube.com/playlist?yv2t=false`;
    - `https://youtube.com/playlist?some=some&yv2t_bot=false`.

### 7. Random media
- **Private chat**:
  - Use the command `/rv` to get random video
  - Use the command `/ra` to get random video

  Supports domains specification to get media only from these sources.\
  Format: `[/command] [domains=youtube.com|youtu.be]`.
  Example:
    - `/rv [domains=youtube.com|youtu.be]`,
    - `/rv [domains=youtube.com]`.
