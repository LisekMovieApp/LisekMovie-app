<p align="center">
  <img src="LisekMovie-Frontend/src/lib/assets/logo_notext.svg" alt="LisekMovie logo" width="140" />
</p>

<h1 align="center">LisekMovie</h1>

<p align="center">
  A fast, self-hosted video-on-demand server for your own films and series.<br />
  Rust + Axum backend · SvelteKit 5 web app · PostgreSQL · FFmpeg
</p>

<p align="center">
  <img alt="Version" src="https://img.shields.io/badge/version-1.0.0--beta-bda24d" />
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.97%20·%20edition%202024-8a7638" />
  <img alt="License" src="https://img.shields.io/badge/license-PolyForm%20Noncommercial-555" />
</p>

---

LisekMovie indexes your movie and TV libraries, fetches metadata and artwork, and streams to any browser, phone or TV — direct play when the device can, a fast HLS remux when only the container is wrong, and a full (optionally GPU-accelerated) transcode when it has to. It is a from-scratch replacement for the classic self-hosted media servers: one small Rust binary, a documented REST API, and a modern web app that works with a mouse, a touchscreen and a TV remote alike.

> **Status: 1.0.0-beta.** Feature-complete for everyday use. The database schema is migrated automatically on start; expect fixes rather than breaking changes until 1.0.0.

## Contents

- [Features](#features)
- [Architecture](#architecture)
- [Quick start with Docker](#quick-start-with-docker)
- [Configuration](#configuration)
- [Using LisekMovie](#using-lisekmovie)
- [Development](#development)
- [License](#license)

## Features

### Playback

- **Per-stream playback plan** — direct play, **remux** (video copied into HLS, only incompatible audio converted) or transcode, chosen for each video, audio and subtitle stream the client can or cannot decode.
- **Fast HLS remux** — far seeks restart FFmpeg at the target segment instead of waiting for the whole file; the integration suite serves a 40-second seek in a few hundredths of a second.
- **Transcode throttling** — FFmpeg pauses once it is `throttle_lead_seconds` ahead of the player and resumes when the player catches up, so an idle pause costs no CPU and no disk.
- **Hardware acceleration** — NVENC/NVDEC, VAAPI and QSV encoding and decoding; **hardware HDR → SDR tone mapping** (`tonemap_cuda` with BT.2390, Dolby Vision, OpenCL, or libplacebo/Vulkan) with [jellyfin-ffmpeg](https://github.com/jellyfin/jellyfin-ffmpeg). Admin → Playback lists what the configured FFmpeg build supports.
- **Subtitles** — text subtitles converted to WebVTT, image subtitles burned in when needed.
- **End time** — the player shows when the title will finish ("Ends at 13:15"); paused, the time moves on with the clock.
- **Resume and "Next up"** — playback position saved per account, the next episode offered at the end credits.
- **Seek-bar previews (trickplay)** — thumbnail sheets generated in the background after each scan; progress under Admin → Thumbnails.
- **Remote-viewer limits** — bitrate caps for viewers outside the local network, with trusted-proxy support so the real client address is used.

### Library and metadata

- **Library scanner** with live progress, queued scan jobs and stored FFprobe results.
- **Metadata plugins** — [TMDB](https://www.themoviedb.org/) and [TheTVDB](https://thetvdb.com/) providers behind one plugin API; provider API keys are stored encrypted (`LISEK_SECRET_KEY`).
- **Fix Match** — search the providers by title and year and pin an item (or a whole series) to the right entry; old artwork is replaced and the item is locked against automatic re-matching.
- **Metadata editor** with per-field locking, optional `.nfo`-style saving of artwork next to the video, and a six-hourly background refresh.
- **Cast & crew** — main actors (with the parts they play), directors and writers from TMDB, profile photos cached locally. A person's page shows the photo, biography, birth and death dates with the age, IMDb/TMDB links and every title of *your* libraries they appear in. Biographies are fetched the first time a page is opened, so scans stay fast.
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
tools/                       end-to-end smoke test against a disposable database
docker-compose.yml           PostgreSQL + server + web app
docker-compose.gpu.yml       NVIDIA GPU override
```

Metadata providers live in their own repositories next to this one — [`LisekMovie-plugin-tmdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-tmdb) and [`LisekMovie-plugin-thetvdb`](https://github.com/LisekMovieApp/LisekMovie-plugin-thetvdb) — so they can be versioned independently of the server.

## Quick start with Docker

Requirements: Docker with Compose v2. For GPU transcoding, an NVIDIA driver and the NVIDIA Container Toolkit (Docker Desktop on Windows: the WSL 2 backend is enough).

The server image is built from the folder that **contains** this repository, because it also compiles the provider plugins. Clone all three side by side:

```bash
mkdir LisekMovie && cd LisekMovie
git clone https://github.com/LisekMovieApp/LisekMovie-app.git
git clone https://github.com/LisekMovieApp/LisekMovie-plugin-tmdb.git
git clone https://github.com/LisekMovieApp/LisekMovie-plugin-thetvdb.git
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

Everything else — transcoding, hardware acceleration, tone mapping, throttling, bitrate limits, resume thresholds — is set in the web app under **Admin → Playback**.

## Using LisekMovie

- **Fix a wrong match** — on a film or series page an administrator chooses **Fix match**, searches by title (and optionally year and provider) and picks the right entry. A series is renamed with all its episodes; the artwork is fetched again.
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

## License

LisekMovie is licensed under the **[PolyForm Noncommercial License 1.0.0](LICENSE)**, with an additional attribution requirement.

In short: you're free to use, fork, modify and self-host LisekMovie — including rebranding your own instance — for any non-commercial purpose, such as running it for yourself, your family or a community. You may not sell access to it or otherwise use it commercially. Any fork or redistribution must keep a visible credit to the original project. See [LICENSE](LICENSE) for the full, binding terms.

This is not legal advice — if licensing terms really matter for what you plan to build, have them reviewed by a lawyer.
