# LisekMovie

A self-hosted video-on-demand media server, built from scratch for speed, a clean architecture, and a modern, Netflix-style interface.

LisekMovie indexes your personal movie and TV libraries, fetches rich metadata, streams and transcodes on the fly, and gives you a fast, good-looking web app to browse and watch from — plus a few things most self-hosted media servers leave out, like built-in playback stats, webhooks, and a couple of small games.

> **Status: early development.** The project is being built in public, in phases. Nothing here is ready to run yet — see [Roadmap](#roadmap) below.

## Why

Most self-hosted media servers carry a decade of accumulated technical debt. LisekMovie is a from-scratch rewrite that aims to be:

- **Fast and light** — a Rust backend with no garbage collector, built for low memory use even while transcoding and scanning large libraries.
- **API-first** — every feature is a documented REST endpoint before it's a UI. The web app is just the first client; native and homebrew clients are a natural extension, not an afterthought.
- **Honest about scope** — telemetry, webhooks, and transcode-safety controls are built into the core server, not bolted on as optional community plugins.
- **Private by default** — playback data about your library and your users never leaves your server.

## Tech stack

| Layer | Choice |
|---|---|
| Backend | [Rust](https://www.rust-lang.org/) + [Axum](https://github.com/tokio-rs/axum) |
| Database | [PostgreSQL](https://www.postgresql.org/) via [sqlx](https://github.com/launchbadge/sqlx) |
| API spec | [utoipa](https://github.com/juhaku/utoipa) (OpenAPI, generated from code) |
| Frontend | [SvelteKit](https://kit.svelte.dev/) + TypeScript + [Tailwind CSS](https://tailwindcss.com/) |
| Transcoding | [FFmpeg](https://ffmpeg.org/) |

## Repository layout

```
LisekMovie-Backend/     Rust workspace (Axum server + domain crates)
LisekMovie-Frontend/    SvelteKit web app
```

Metadata provider integrations (TMDB, TheTVDB, OMDb, ...) live in their own repositories under the same organization, so they can ship and version independently of the core server.

## Roadmap

Development runs in phases, roughly:

1. Repository & tooling scaffolding
2. Database schema & first-run setup wizard
3. Backend core & authentication
4. Frontend shell & UI
5. Library scanner, metadata editor & locking
6. Native features — stats, transcode protection, webhooks/notifications, games
7. External metadata provider plugins

## Development

The backend builds without a running database: SQL written with `sqlx` query macros is checked against the committed `LisekMovie-Backend/.sqlx/` cache (`SQLX_OFFLINE=true` in `LisekMovie-Backend/.cargo/config.toml`). After adding or changing such a query, refresh the cache against a database migrated to the latest schema:

```
cd LisekMovie-Backend
sqlx migrate run --source lisek-db/migrations
cargo sqlx prepare --workspace
```

Database-backed tests (`lisek-db/tests/`) run when `LISEK_TEST_DATABASE_URL` points at a PostgreSQL role allowed to create databases, e.g. `postgres://lisekmovie:lisekmovie@localhost/postgres`; otherwise they are skipped.

## License

LisekMovie is licensed under the **[PolyForm Noncommercial License 1.0.0](LICENSE)**, with an additional attribution requirement.

In short: you're free to use, fork, modify, and self-host LisekMovie — including rebranding your own instance — for any non-commercial purpose, such as running it for yourself, your family, or a community. What you may not do is sell access to it or otherwise use it commercially. Any fork or redistribution must keep a visible credit to the original project. See the [LICENSE](LICENSE) file for the full, binding terms.

This is not legal advice — if you plan to build on LisekMovie in a way where licensing terms really matter to you, have it reviewed by a lawyer.
