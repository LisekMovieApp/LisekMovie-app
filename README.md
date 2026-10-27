<!-- Logo: replace the image below with the final artwork (SVG or a 512 px PNG). -->
<p align="center">
  <img src="LisekMovie-Frontend/src/lib/assets/logo_notext.svg" alt="LisekMovie logo" width="140" />
</p>

<h1 align="center">LisekMovie</h1>

<p align="center">
  <strong>Your films and series, streamed beautifully — from your own server.</strong><br />
  A fast, self-hosted video-on-demand server: Rust backend · SvelteKit 5 web app · PostgreSQL · FFmpeg
</p>

<p align="center">
  <a href="https://github.com/LisekMovieApp/LisekMovie-app/releases"><img alt="Latest release" src="https://img.shields.io/github/v/release/LisekMovieApp/LisekMovie-app?include_prereleases&sort=semver&label=release&color=bda24d" /></a>
  <img alt="Version" src="https://img.shields.io/badge/version-1.0.0--beta-bda24d" />
  <img alt="Rust" src="https://img.shields.io/badge/Rust-1.97%20·%20Axum-b7410e?logo=rust&logoColor=white" />
  <img alt="Svelte" src="https://img.shields.io/badge/Svelte-5%20·%20SvelteKit-ff3e00?logo=svelte&logoColor=white" />
  <img alt="PostgreSQL" src="https://img.shields.io/badge/PostgreSQL-16%2B-4169e1?logo=postgresql&logoColor=white" />
  <img alt="Docker" src="https://img.shields.io/badge/Docker-Compose-2496ed?logo=docker&logoColor=white" />
  <img alt="Platforms" src="https://img.shields.io/badge/platform-Linux%20·%20Windows-555" />
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-MIT-555" /></a>
</p>

<p align="center">
  <a href="#installation">Installation</a> ·
  <a href="#updating">Updating</a> ·
  <a href="#features">Features</a> ·
  <a href="#configuration">Configuration</a> ·
  <a href="#development">Development</a>
</p>

---

LisekMovie indexes your movie and TV libraries, fetches metadata and artwork, and streams to any browser, phone or TV — direct play when the device can, a fast HLS remux when only the container is wrong, and a full (optionally GPU-accelerated) transcode when it has to. It is a from-scratch replacement for the classic self-hosted media servers: one small Rust binary, a documented REST API, and a modern web app that works with a mouse, a touchscreen and a TV remote alike.

> **Status: 1.0.0-beta.** Feature-complete for everyday use. The database schema is migrated automatically on start; expect fixes rather than breaking changes until 1.0.0.

## Highlights

- 🦀 **High-performance Rust backend** — Axum on tokio, PostgreSQL through compile-time-checked `sqlx` queries, a single binary with a small memory footprint and an OpenAPI-documented REST API.
- 🎛️ **Smart transcoding** — a per-stream plan (direct play, HLS remux or transcode) with hardware acceleration on **Intel Quick Sync (QSV)**, **VA-API** (Intel/AMD) and **NVIDIA NVENC/NVDEC**, hardware HDR → SDR tone mapping, and **multi-GPU load balancing**: a weighted pool spreads sessions over several cards and architectures (e.g. two streams on an RTX, one on the iGPU) with per-device session limits and codec switches.
- 📱 **Progressive Web App** — install it on iOS and Android from the browser; it opens full screen like a native app.
- 📺 **Spatial navigation for TV** — every screen works with the arrow keys of a TV remote: focus scopes, carousels, the search overlay and the A–Z picker included.
- 🎭 **Cast & crew database** — actors with their roles, directors and writers from TMDB, with photos, biographies and every title of *your* libraries they appear in.
- ⏱️ **Dynamic ETA** — the player and the detail pages show when a title will finish ("Ends at 23:41"), and the time keeps moving while playback is paused.
- 🔎 **And more** — global search, watch statistics with Discord-ready bot reports, parental controls, chapters as scenes, seek-bar previews, collections, webhooks — see [all features](#features).

## Contents

- [Highlights](#highlights)
- [Installation](#installation)
- [Updating](#updating)
- [Features](#features)
- [Architecture](#architecture)
- [Manual Docker setup](#manual-docker-setup)
- [Configuration](#configuration)
- [Logs](#logs)
- [Using LisekMovie](#using-lisekmovie)
- [Development](#development)
- [Publishing a release](#publishing-a-release)
- [License](#license)

## Installation

### Linux: the interactive installer

Supported out of the box on **Debian**, **Ubuntu** and **Arch Linux** (x86_64 and aarch64). On other distributions the installer explains that support is planned and asks whether to continue at your own risk.

```bash
curl -fsSLO https://raw.githubusercontent.com/LisekMovieApp/LisekMovie-app/main/install-linux.sh
sudo bash install-linux.sh
```

The installer asks for the language (English or Polski) and how to install:

| # | Method | What it does | Best for |
|---|---|---|---|
| 1 | **Build from source** | Installs build tools, Rust (in `/opt/lisekmovie/toolchain`, not system-wide), Node.js 24 (when missing or older than 20) and FFmpeg; clones the app and the plugins; builds a release binary and the web app. | Contributors, unusual CPUs, trying the newest `main` |
| 2 | **Docker Compose** *(recommended)* | Installs Docker and the Compose plugin if needed, prepares `.env` (media folder, port, a random database password, NVIDIA / Intel GPU overlays when a GPU is found) and runs `docker compose up -d --build`. | Most servers and NAS boxes |
| 3 | **Release binaries** | Downloads the latest package from [GitHub Releases](https://github.com/LisekMovieApp/LisekMovie-app/releases), verifies its SHA-256 checksum, and installs Node.js and FFmpeg when missing. | Fast installs without compiling |

Methods 1 and 3 also offer to set up a local PostgreSQL database (a dedicated role with a random password and the `pg_trgm` extension) and install two systemd services, `lisekmovie-server` and `lisekmovie-web`, running as the unprivileged `lisek` user (in the `video`/`render` groups for GPU access).

When it finishes, open `http://<server>:8096` and create the administrator account.

<details>
<summary>Where everything goes</summary>

| Path | Contents | Touched by updates? |
|---|---|---|
| `/opt/lisekmovie/app` | the program (`bin/lisek-server`, `web/`) | replaced; the previous version is kept in `app.prev` |
| `/opt/lisekmovie/src` | git checkouts (source and Docker methods) | `git pull --ff-only` |
| `/opt/lisekmovie/install.conf` | method and version, read by the updater | version line only |
| `/var/lib/lisekmovie/.env` | server settings, database address, `LISEK_SECRET_KEY` | **never** |
| `/var/lib/lisekmovie/config` | posters, artwork, trickplay thumbnails, cache | **never** |
| `/etc/lisekmovie/web.env` | port and address of the web app | **never** |
| `/opt/lisekmovie/src/LisekMovie-app/.env` | settings of the Docker method | **never** |
| Docker volumes `lisekmovie-postgres-data`, `lisekmovie-data` | database, settings, artwork | **never** |

</details>

Unattended installs: `sudo LISEK_LANG=en LISEK_METHOD=docker LISEK_ASSUME_YES=1 bash install-linux.sh` (methods: `source`, `docker`, `release`). `bash install-linux.sh --help` lists every option. The full log of a run is in `/tmp/lisekmovie-install.log`.

### Windows, macOS and everything else

Use Docker (Docker Desktop with the WSL 2 backend on Windows) as described in [Manual Docker setup](#manual-docker-setup), or build from source following [Development](#development).

## Updating

```bash
sudo bash /opt/lisekmovie/update-linux.sh          # update now
sudo bash /opt/lisekmovie/update-linux.sh --check  # only check (exit code 10 = update available)
```

The updater detects how LisekMovie was installed and updates it the same way:

- **Source** — `git pull --ff-only` on the app and every provider plugin (a plugin added by a newer version is cloned), a fresh release build next to the running version, then a switch.
- **Docker** — `git pull` (cloning new plugins), `docker compose pull`, `docker compose up -d --build`. It also works in a manual clone: run `sudo bash update-linux.sh` inside `LisekMovie-app`.
- **Release** — downloads the newest release, verifies the checksum, then switches.

**Your data is safe by design.** The updater never writes, replaces or deletes `.env` (it verifies its SHA-256 before and after), never drops or resets the database (the new version only applies its migrations), never runs `docker compose down -v`, and never removes the data folder with posters, thumbnails and trickplay sheets. Before switching versions it writes a compressed `pg_dump` to `/var/backups/lisekmovie` (the newest five are kept). For source and release installs, if the new version does not pass its health check, the previous one is put back automatically. Local changes to the git checkouts stop the update instead of being overwritten, and a PostgreSQL major-version change in the compose file is refused rather than started on an incompatible data folder.

Options: `--yes` (no questions — for cron), `--force` (reinstall the same version), `--no-backup`, `--lang=en|pl`.

## Features

### Playback

- **Per-stream playback plan** — direct play, **remux** (video copied into HLS, only incompatible audio converted) or transcode, chosen for each video, audio and subtitle stream the client can or cannot decode.
- **Fast HLS remux** — far seeks restart FFmpeg at the target segment instead of waiting for the whole file; the integration suite serves a 40-second seek in a few hundredths of a second.
- **Transcode throttling** — FFmpeg pauses once it is `throttle_lead_seconds` ahead of the player and resumes when the player catches up, so an idle pause costs no CPU and no disk.
- **Hardware acceleration** — NVENC/NVDEC, VAAPI and QSV encoding and decoding; **hardware HDR → SDR tone mapping** (`tonemap_cuda` with BT.2390, Dolby Vision, OpenCL, or libplacebo/Vulkan) with [jellyfin-ffmpeg](https://github.com/jellyfin/jellyfin-ffmpeg). Admin → Playback lists what the configured FFmpeg build supports.
- **Subtitles** — text subtitles converted to WebVTT, image subtitles burned in when needed.
- **End time** — the player shows when the title will finish ("Ends at 13:15"); paused, the time moves on with the clock.
- **Pick before you play** — the detail page lists the file's picture ("4K HEVC HDR"), audio tracks and subtitles; Play starts with the chosen lektor and subtitles.
- **Scenes** — chapters of MKV/MP4 files (or one scene every 10 minutes when a file has none) as a carousel of thumbnails, cut by FFmpeg on first view; a scene opens the player at its exact time.
- **Resume and "Next up"** — playback position saved per account, the next episode offered at the end credits.
- **Seek-bar previews (trickplay)** — thumbnail sheets generated in the background after each scan; progress under Admin → Thumbnails.
- **Remote-viewer limits** — bitrate caps for viewers outside the local network, with trusted-proxy support so the real client address is used.

### Library and metadata

- **Library scanner** with live progress, queued scan jobs and stored FFprobe results.
- **Metadata plugins** — [TMDB](https://www.themoviedb.org/), [TheTVDB](https://thetvdb.com/), [IMDb](https://www.imdb.com/), [OMDb](https://www.omdbapi.com/), [AniList](https://anilist.co/), [aniDB](https://anidb.net/) and [IMVDb](https://imvdb.com/) providers behind one plugin API (aniSearch, VGMdb and TheAudioDB registered for the coming anime and audio work); Admin → Plugins shows what each covers and whether it needs a key, and every library can order them its own way (anime sources first in an anime library). Provider API keys are stored encrypted (`LISEK_SECRET_KEY`).
- **Fix Match** — search the providers by title and year, or type an id straight away (`tt0425622` for IMDb, `tvdb:60620`, `anilist:21`, a number with one provider picked, or a provider page link), and pin an item (or a whole series) to the right entry; old artwork is replaced and the item is locked against automatic re-matching.
- **Rich details** — original title, tagline, TMDB rating, runtime, genres, tags (keywords), studios, directors, writers and IMDb/TheTVDB/TMDB links.
- **Metadata editor** with per-field locking, optional `.nfo`-style saving of artwork next to the video, and a six-hourly background refresh.
- **Cast & crew** — main actors (with the parts they play), directors and writers from TMDB, profile photos cached locally. A person's page shows the photo, biography, birth and death dates with the age, IMDb/TMDB links and every title of *your* libraries they appear in. Biographies are fetched the first time a page is opened, so scans stay fast.
- **Series pages** — season cards (season poster, episodes left to watch, play / watched / favourite on hover or remote focus) and a season page listing every episode with its still, title, runtime, score, end time and synopsis; "Information" on an episode offers the same audio and subtitle picks as a film.
- **Jellyfin folder layout** — a series' artwork is saved in the series folder (`poster.jpg`, `backdrop.jpg`, `logo.png`, `season01-poster.jpg` ...); season folders keep only the videos, episode stills stay in the server's cache.
- **Global search** — the magnifier in the top bar (or `Ctrl+K` / `/`) opens a full-screen search across films, series, episode titles, tags, genres, original titles, people and collections, grouped into rows and fully usable with a TV remote; backed by `pg_trgm` indexes in PostgreSQL.
- **Alpha picker** — library grids of posters with `#` and A–Z: a column on the right on desktop and TV (reachable with the remote), a strip on tablets, a "Jump to…" menu on phones; a letter loads the right page and focuses its first title.
- **Watch statistics** — an admin dashboard over every playback session: watch time per day and hour, plays, active users, completion and where viewers stop, by platform (Windows, macOS, Linux, Android, iOS, Smart TV, from the User-Agent), device, user and content type, plus rankings of users, films and series.
- **Bot reports** — `GET /api/stats/summary?period=daily|weekly` with a read-only `X-Lisek-Stats-Key`: the day's or week's top films and series with watch time ("1h 38m"), plays and unique viewers; `format=discord` returns a ready Discord webhook body.
- **Hardware diagnostics and multi-GPU transcoding** — one click surveys the CPU, NVIDIA/Intel/AMD GPUs (nvidia-smi, VA-API, Vulkan, WMI on Windows) and test-encodes with every hardware encoder; transcodes are spread over several NVIDIA GPUs (fewest sessions, then most free VRAM).
- **Intel Quick Sync and VA-API transcoding pool** — Intel GPUs (Quick Sync: `-hwaccel qsv`, `h264_qsv`/`hevc_qsv`, `scale_qsv`) and Intel/AMD GPUs through VA-API (Linux) transcode next to NVIDIA cards; in the admin panel each device gets a switch, a share of sessions, a session limit and the codecs it decodes and encodes (`docker-compose.intel.yml` passes `/dev/dri` into the container).
- **Collections** — films of one series (TMDB collections such as Star Wars) are linked automatically; library pages can fold them into a single card.

### Accounts and access

- Administrator and user roles, generated first passwords with enforced change, session revocation, disabling accounts, login rate limiting.
- **Parental controls** — per account: allowed libraries, a maximum age rating (from TMDB certifications or set by hand) and hiding unrated titles. Administrators are never restricted.
- **Original-file downloads** — a "Download" button on every film and episode streams the original file with a readable name (`Title_S01E05.mkv`) and HTTP range support; it can be turned off per account.

### Interface

- **SvelteKit 5** web app with a Netflix-style home page, hero banner, carousels and detail pages, in English and Polish.
- **TV remote navigation** — full spatial (arrow-key) navigation with focus scopes, so the app works on a TV browser or with a remote.
- **Installable PWA** — add to the home screen on iOS and Android; opens full screen, with an offline page and a service worker that only caches the app shell.

### Built in, not bolted on

- **Playback statistics** and telemetry per account — stored on your server only.
- **Webhooks** (generic JSON, Discord, Slack) with retries and a delivery log.
- **Browser notifications** for newly added titles over a WebSocket.
- **OpenAPI** documentation generated from the code (utoipa), and a couple of small **games** for the waiting room.

## Architecture

```mermaid
flowchart LR
    Browser["Browser · phone · TV<br/>(SvelteKit 5 app, PWA)"]
    Web["lisek-web<br/>Node adapter, SSR"]
    Server["lisek-server<br/>Rust · Axum · tokio"]
    PG[("PostgreSQL")]
    FF["FFmpeg / FFprobe<br/>(jellyfin-ffmpeg)"]
    Media[/"Media folders<br/>(read-only)"/]
    Providers["TMDB · TheTVDB"]

    Browser -- "HTTP / HLS / WebSocket" --> Web
    Web -- "/api proxy" --> Server
    Server --> PG
    Server --> FF
    FF --> Media
    Server --> Media
    Server -- "metadata plugins" --> Providers
```

| Layer | Technology |
|---|---|
| HTTP server | [Rust](https://www.rust-lang.org/) (edition 2024) + [Axum](https://github.com/tokio-rs/axum) on [tokio](https://tokio.rs/) |
| Database | [PostgreSQL](https://www.postgresql.org/) via [sqlx](https://github.com/launchbadge/sqlx), migrations in `lisek-db/migrations` |
| API description | [utoipa](https://github.com/juhaku/utoipa) — OpenAPI generated from the handlers, TypeScript types generated from it |
| Media engine | [FFmpeg](https://ffmpeg.org/) 6.1+ (HLS remux and transcode, trickplay, probing) |
| Web app | [SvelteKit](https://svelte.dev/) with Svelte 5 runes, TypeScript, [Tailwind CSS](https://tailwindcss.com/) 4, [hls.js](https://github.com/video-dev/hls.js) |

### Repository layout

```
LisekMovie-Backend/          Rust workspace
  lisek-server/              Axum application: routes, auth, playback, scans, enrichment
  lisek-core/                shared domain types and settings
  lisek-db/                  PostgreSQL access and migrations
  lisek-playback/            playback planning and the HLS engine
  lisek-library/             filesystem scanner and FFprobe
  lisek-metadata/            provider registry: ordering, overrides, locks
  lisek-plugin-api/          the MetadataProvider trait implemented by plugins
  lisek-auth/                login, tokens, permissions
  lisek-telemetry/           playback progress reports and statistics rules
  lisek-notifications/       webhooks and browser notifications
  lisek-games/               leaderboards
LisekMovie-Frontend/         SvelteKit web app
tools/                       end-to-end smoke tests, release packaging (package-release.sh)
install-linux.sh             interactive Linux installer (source / Docker / release)
update-linux.sh              updater for every install method
docker-compose.yml           PostgreSQL + server + web app
docker-compose.gpu.yml       NVIDIA GPU override
docker-compose.intel.yml     Intel / AMD GPU override (/dev/dri)
.github/workflows/           release builds for linux-x86_64 and linux-aarch64
```

Metadata providers live in their own repositories next to this one, so they can be versioned independently of the server. Each implements `lisek_plugin_api::MetadataProvider` and is compiled in as a path dependency (`[workspace.dependencies]` in `LisekMovie-Backend/Cargo.toml`):

| Repository | Provider | Covers | API key | Status |
|---|---|---|---|---|
| [`LisekMovie-plugin-tmdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-tmdb) | TMDB | movies, series, anime | required | full |
| [`LisekMovie-plugin-thetvdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-thetvdb) | TheTVDB | movies, series, anime | required | basic |
| [`LisekMovie-plugin-imdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-imdb) | IMDb | movies, series | none | basic |
| [`LisekMovie-plugin-omdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-omdb) | OMDb | movies, series | required (free) | basic |
| [`LisekMovie-plugin-anilist`](https://github.com/LisekMovieApp/LisekMovie-plugin-anilist) | AniList | anime | none | basic |
| [`LisekMovie-plugin-anidb`](https://github.com/LisekMovieApp/LisekMovie-plugin-anidb) | aniDB | anime | registered client | basic (lookup by id) |
| [`LisekMovie-plugin-anisearch`](https://github.com/LisekMovieApp/LisekMovie-plugin-anisearch) | aniSearch | anime | — | skeleton |
| [`LisekMovie-plugin-imvdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-imvdb) | IMVDb | music videos | required (free) | basic |
| [`LisekMovie-plugin-vgmdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-vgmdb) | VGMdb | music | — | skeleton (audio libraries) |
| [`LisekMovie-plugin-audiodb`](https://github.com/LisekMovieApp/LisekMovie-plugin-audiodb) | TheAudioDB | music | optional | skeleton (audio libraries) |

New providers start disabled; turn them on and order them in **Admin → Plugins**, or per library.

## Manual Docker setup

Requirements: Docker with Compose v2. For GPU transcoding, an NVIDIA driver and the NVIDIA Container Toolkit (Docker Desktop on Windows: the WSL 2 backend is enough).

The server image is built from the folder that **contains** this repository, because it also compiles the provider plugins. Clone the app and every plugin side by side:

```bash
mkdir LisekMovie && cd LisekMovie
for repo in app plugin-tmdb plugin-thetvdb plugin-imdb plugin-omdb plugin-anidb plugin-anilist \
            plugin-anisearch plugin-imvdb plugin-vgmdb plugin-audiodb; do
  git clone "https://github.com/LisekMovieApp/LisekMovie-$repo.git"
done
cd LisekMovie-app

cp .env.example .env     # set at least POSTGRES_PASSWORD and MEDIA_DIR
docker compose up -d --build

# with an NVIDIA GPU (NVENC, CUDA tone mapping):
docker compose -f docker-compose.yml -f docker-compose.gpu.yml up -d --build
```

Open `http://<host>:8096`. The first-run wizard only asks for the administrator account — the database comes from the compose file. Then add libraries in **Admin → Libraries** using paths *inside* the container, i.e. under `/media` (your `MEDIA_DIR` is mounted there read-only).

Back up both volumes together: `lisekmovie-postgres-data` (the database) and `lisekmovie-data` (settings, posters, trickplay sheets and the generated `LISEK_SECRET_KEY`).

### HTTPS and reverse proxies

Put a reverse proxy with a certificate (Caddy, Traefik, nginx) in front of port `8096` and set `LISEK_TRUST_PROXY=1`. Android only offers to install the PWA over HTTPS. Enter the compose subnet (`LISEK_SUBNET`) under Admin → Playback → trusted proxies so remote-viewer limits see real client addresses.

## Configuration

All settings live in `.env` next to `docker-compose.yml`; [`.env.example`](.env.example) documents each one.

| Variable | Purpose |
|---|---|
| `POSTGRES_PASSWORD` | Database password (used by the `postgres` service and the server). |
| `MEDIA_DIR` | Your films and series on the host, mounted at `/media`. Windows: e.g. `D:/Filmy`. |
| `LISEK_HTTP_PORT` | Port of the web app on the host (default `8096`). |
| `LISEK_SECRET_KEY` | 32-byte key that encrypts provider API keys; generated on first start when empty. |
| `TMDB_API_KEY` / `TVDB_API_KEY` | Metadata provider keys (TMDB has a built-in default; TheTVDB stays off without a key). |
| `LISEK_FFMPEG_FLAVOR` | `jellyfin` (default, hardware tone mapping) or `debian` (the distribution's FFmpeg). |
| `LISEK_TRICKPLAY` | `0` turns off seek-bar preview generation. |
| `LISEK_TRUST_PROXY` | `1` behind another reverse proxy with TLS. |
| `TZ` | Time zone of the containers. |
| `LISEK_LOG_LEVEL` | Level of the log files (`info` by default; `debug` for more detail). |
| `LISEK_PROVIDER_URL_<KEY>` | Another base URL for one metadata provider (`LISEK_PROVIDER_URL_IMDB=...`): a mirror, a proxy or a test server. |

Everything else — transcoding, hardware acceleration, tone mapping, throttling, bitrate limits, resume thresholds — is set in the web app under **Admin → Playback**.

## Logs

The server writes one log file a day to `<config>/logs/lisekmovie.YYYY-MM-DD.log` (JSON lines, a new file at midnight UTC) and deletes files older than 30 days. In Docker that folder is inside the `lisekmovie-data` volume; a native install keeps it in `/var/lib/lisekmovie/config/logs`.

Every entry is tagged with its source:

- **Backend** — `lisek-server`, the scanner, metadata providers, the database;
- **FFmpeg / FFprobe** — the tools' warnings and errors (`[FFMPEG]`, `[FFPROBE]`), e.g. a file that cannot be read or a GPU that fails to open;
- **Frontend** — errors in viewers' browsers (`[FRONTEND]`), sent by the web app to `POST /api/logs/client`.

**Admin → Logs** shows them as a console: pick a day, show or hide sources and levels (ERROR, WARN, INFO, DEBUG), search, follow today's file live and download the raw file.

## Using LisekMovie

- **Fix a wrong match** — on a film or series page an administrator chooses **Fix match**, searches by title (and optionally year and provider) — or by id: an IMDb id (`tt0425622`) is resolved by IMDb, OMDb, TMDB and TheTVDB; `tvdb:60620`, `tmdb:603`, `anilist:21`, `anidb:69` or a link to the title's page go to that provider; a plain number is looked up at the provider picked in the form (and also searched as a title, since "1917" is a film) — and picks the right entry. A series is renamed with all its episodes; the artwork is fetched again.
- **Download** — **Download** on a film page, or the download icon next to an episode, saves the original file. To keep an account to streaming only, untick *Allow downloading original files* under Admin → Users → the account → Access control.
- **Parental controls** — the same Access control panel limits libraries and age ratings, with quick *child* and *teen* presets.
- **Install on a phone** — iPhone/iPad (Safari): Share → *Add to Home Screen*. Android (Chrome): account menu → *Install app*.
- **API** — every feature is a REST endpoint; the OpenAPI document is served at `/api/openapi.json` and checked into `LisekMovie-Frontend/src/lib/api/openapi.json`.

## Development

Requirements: Rust 1.97+, Node.js 24, PostgreSQL 16+, FFmpeg 6.1+.

```bash
# database only
docker compose up -d postgres
cp .env.example .env     # the defaults match the compose database

# backend (http://127.0.0.1:8080)
cd LisekMovie-Backend
cargo run -p lisek-server

# web app (http://localhost:5173, proxies /api to the backend)
cd ../LisekMovie-Frontend
npm ci
npm run dev
```

### Checks

```bash
# backend
cd LisekMovie-Backend
cargo clippy --workspace --all-targets -- -D warnings
LISEK_TEST_DATABASE_URL=postgres://lisekmovie:lisekmovie@localhost/postgres cargo test --workspace

# web app
cd ../LisekMovie-Frontend
npm run check && npx eslint . && npm test && npm run build

# end to end: a disposable database, a real scan, HLS playback in headless Chromium
python3 tools/smoke_test.py --server LisekMovie-Backend/target/debug/lisek-server \
  --psql psql --database-url postgres://lisekmovie:lisekmovie@localhost/postgres \
  --ffmpeg ffmpeg --ffprobe ffprobe --node node \
  --hlsjs LisekMovie-Frontend/node_modules/hls.js/dist/hls.min.js
```

Database-backed tests run only when `LISEK_TEST_DATABASE_URL` points at a role allowed to create databases; otherwise they are skipped. `lisek-playback/tests/hls_engine.rs` uses the `ffmpeg` on `PATH` and is skipped without it.

### SQL and the offline cache

The backend builds without a running database: queries written with the `sqlx` macros are checked against the committed `LisekMovie-Backend/.sqlx/` cache (`SQLX_OFFLINE=true` in `.cargo/config.toml`). After adding or changing such a query, refresh the cache against a database migrated to the latest schema:

```bash
cd LisekMovie-Backend
sqlx migrate run --source lisek-db/migrations
cargo sqlx prepare --workspace
```

### API types

After changing a route, regenerate the OpenAPI document and the TypeScript types the web app uses:

```bash
cd LisekMovie-Frontend
npm run gen:api      # writes src/lib/api/openapi.json and schema.d.ts
```

## Publishing a release

Release packages are built by GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)) when a version tag is pushed:

```bash
git tag v1.0.0-beta
git push origin v1.0.0-beta
```

The workflow checks out the app and every provider plugin side by side, builds `lisek-server` and the web app on Ubuntu 22.04 (glibc 2.35, so the binaries run on Debian 12, Ubuntu 22.04+ and Arch) for x86_64 and aarch64, packs them with [`tools/package-release.sh`](tools/package-release.sh) into `lisekmovie-<version>-linux-<arch>.tar.gz` plus a `.sha256` file, and attaches them to the GitHub release. Tags with a suffix (`-beta`, `-rc.1`) become pre-releases. The installer and the updater pick up the newest release, pre-releases included.

## License

LisekMovie — the app and every `LisekMovie-plugin-*` provider extension — is released under the **[MIT License](LICENSE)**: use, copy, modify, fork and distribute it, also commercially, as long as the copyright notice and the license text stay with every copy or substantial portion.

The software is provided "as is", without warranty of any kind; see [LICENSE](LICENSE) for the full terms.
