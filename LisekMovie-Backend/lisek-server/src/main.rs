//! LisekMovie server binary.
//!
//! Startup: load `.env`, resolve the secret key, connect to the saved
//! database (or start in setup mode), spawn the background workers, serve
//! the routes from `routes.rs`.
//!
//! `lisek-server --print-openapi` prints the OpenAPI document and exits
//! without touching a database; the frontend generates its API types from
//! that output (`npm run gen:api`).

mod admin;
mod auth;
mod users;
mod background_refresh;
mod collections;
mod env_file;
mod media;
mod playback;
mod games;
mod notifications;
mod browser_notifications;
mod openapi;
mod metadata_enrichment;
mod provider_registry;
mod rate_limit;
mod routes;
mod scan_jobs;
mod setup;
mod state;
mod telemetry;
mod trickplay;

use std::sync::Arc;
use std::time::Duration;

use axum::http::{HeaderValue, Method, header};
use lisek_auth::secrets::SecretBox;
use tower_http::cors::{AllowOrigin, CorsLayer};

use state::{AppState, ServerMode};

#[tokio::main]
async fn main() {
    if std::env::args().any(|arg| arg == "--print-openapi") {
        let state = Arc::new(AppState::new(
            ServerMode::SetupRequired,
            SecretBox::from_encoded_key(&SecretBox::generate_encoded_key()).expect("valid key"),
        ));
        let (_, api) = routes::build(&state);
        println!("{}", api.to_pretty_json().expect("OpenAPI document serializes"));
        return;
    }

    // No RUST_LOG set → default to "info" so a plain `cargo run` actually
    // shows startup logs, instead of silently sitting at the default
    // "errors only" level.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Ignored if `.env` doesn't exist yet — that's the normal state before
    // the First-Run Wizard has ever completed.
    dotenvy::dotenv().ok();

    // Resolved and logged here, up front, rather than lazily on whichever
    // request happens to touch it first — an admin comparing this line
    // against the folder they expect posters/backdrops to land in is the
    // fastest way to catch a launch-directory mismatch (see
    // `state::config_dir`'s own doc comment for exactly what that bug
    // looked like) before it costs another round of "the image is 404ing
    // and I don't know why."
    let _ = state::config_dir();

    let mode = detect_initial_mode().await;
    match &mode {
        ServerMode::SetupRequired => {
            tracing::info!(
                "No database configured yet — starting in setup mode. \
                 Open the web app to run the First-Run Wizard."
            );
        }
        ServerMode::DatabaseError(reason) => {
            tracing::error!("A database is configured but not reachable: {reason}");
        }
        ServerMode::Ready(_) => {
            tracing::info!("Connected to the configured database.");
        }
    }

    let secrets = load_secret_key();
    if let ServerMode::Ready(pool) = &mode {
        seal_legacy_provider_keys(pool, &secrets).await;
    }
    let state = Arc::new(AppState::new(mode, secrets));

    // Faza 5 completion's "Zadanie w tle": one detached long-running task,
    // started here so it can't delay the server's own startup or block a
    // request — see background_refresh.rs's own doc comment for exactly
    // what it does and how it stays safe around locked items/fields.
    tokio::spawn(background_refresh::run(state.clone()));
    tokio::spawn(notifications::run(state.clone()));
    tokio::spawn(scan_jobs::run_worker(state.clone()));
    tokio::spawn(trickplay::run_worker(state.clone()));
    tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            // Players report progress every 10 s; 90 s of silence means the
            // tab is gone.
            lisek_playback::session::stop_idle(std::time::Duration::from_secs(90)).await;
        }
    });

    let (router, _) = routes::build(&state);
    let mut app = router.with_state(state);
    if let Some(cors) = cors_layer() {
        app = app.layer(cors);
    }

    let bind_addr = std::env::var("LISEK_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind to {bind_addr}: {err}"));

    tracing::info!(
        "LisekMovie backend listening on http://{}",
        listener.local_addr().unwrap()
    );

    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.expect("server error");
}

/// Looks for a previously saved `DATABASE_URL` and tries to connect. No
/// saved URL at all means the wizard has never run; a saved URL that fails
/// to connect is a real operational problem, not a fresh install, so it's
/// reported distinctly (see [`ServerMode`]).
async fn detect_initial_mode() -> ServerMode {
    let Ok(database_url) = std::env::var("DATABASE_URL") else {
        return ServerMode::SetupRequired;
    };

    match lisek_db::connect(&database_url).await {
        Ok(pool) => match lisek_db::run_migrations(&pool).await {
            Ok(()) => ServerMode::Ready(pool),
            Err(err) => ServerMode::DatabaseError(err.to_string()),
        },
        Err(err) => ServerMode::DatabaseError(err.to_string()),
    }
}

/// `LISEK_SECRET_KEY` from the environment / `.env`. On first start a key
/// is generated and appended to `.env` next to `DATABASE_URL`: back the two
/// up together. An invalid value stops the server — silently replacing it
/// would make every stored provider key unreadable.
fn load_secret_key() -> SecretBox {
    let configured = std::env::var("LISEK_SECRET_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty())
        // An empty variable (docker compose passing `LISEK_SECRET_KEY=` from
        // a copied .env.example) must not hide the key this server
        // generated and saved in its own .env on an earlier start: dotenvy
        // never overrides a variable that is already set, even to "".
        .or_else(saved_secret_key);
    match configured {
        Some(value) => SecretBox::from_encoded_key(&value)
            .unwrap_or_else(|err| panic!("LISEK_SECRET_KEY is invalid: {err}")),
        None => {
            let key = SecretBox::generate_encoded_key();
            match env_file::save_var("LISEK_SECRET_KEY", &key) {
                Ok(()) => tracing::info!(
                    "generated LISEK_SECRET_KEY and saved it to .env; \
                     back it up together with the database"
                ),
                Err(err) => tracing::error!(
                    %err,
                    "generated LISEK_SECRET_KEY but could not save it to .env; provider API \
                     keys entered now will be unreadable after a restart"
                ),
            }
            SecretBox::from_encoded_key(&key).expect("generated key is valid")
        }
    }
}

/// A non-empty `LISEK_SECRET_KEY` saved in `.env` in the working directory.
fn saved_secret_key() -> Option<String> {
    dotenvy::from_filename_iter(".env")
        .ok()?
        .filter_map(Result::ok)
        .find(|(key, value)| key == "LISEK_SECRET_KEY" && !value.trim().is_empty())
        .map(|(_, value)| value)
}

/// Encrypts provider API keys saved in plaintext by earlier versions.
async fn seal_legacy_provider_keys(pool: &lisek_db::PgPool, secrets: &SecretBox) {
    match lisek_db::seal_legacy_provider_keys(pool, |provider, plain| secrets.seal(plain, provider)).await {
        Ok(0) => {}
        Ok(count) => tracing::info!(count, "encrypted provider API keys stored in plaintext"),
        Err(err) => tracing::error!(%err, "could not encrypt plaintext provider API keys"),
    }
}

/// CORS is off by default: the web app is served from the same origin as
/// the API (Vite proxy in development, one reverse proxy in production),
/// and native clients do not use CORS. A browser app on another origin is
/// allowed only when listed in `LISEK_CORS_ORIGINS` (comma-separated, e.g.
/// `https://lisek.example.com,http://localhost:5173`). Auth travels in the
/// `Authorization` header, never cookies, so credentials stay disabled.
fn cors_layer() -> Option<CorsLayer> {
    let raw = std::env::var("LISEK_CORS_ORIGINS").ok()?;
    let origins: Vec<HeaderValue> = raw
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .filter_map(|origin| match origin.parse::<HeaderValue>() {
            Ok(value) if origin.starts_with("http://") || origin.starts_with("https://") => Some(value),
            _ => {
                tracing::error!(origin, "ignoring invalid entry in LISEK_CORS_ORIGINS");
                None
            }
        })
        .collect();
    if origins.is_empty() {
        return None;
    }
    tracing::info!(origins = %raw, "CORS enabled for listed origins");
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::RANGE])
            .expose_headers([
                header::CONTENT_RANGE,
                header::ACCEPT_RANGES,
                header::CONTENT_LENGTH,
                header::RETRY_AFTER,
            ])
            .max_age(Duration::from_secs(600)),
    )
}
