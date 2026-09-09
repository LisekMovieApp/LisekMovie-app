//! LisekMovie server binary.
//!
//! Faza 2 added detecting whether a database connection has already been
//! saved from a completed First-Run Wizard run. Faza 3 adds login/sessions
//! and splits the router into a public group and an `AuthUser`-protected
//! group — see `auth.rs` for what "protected" means here. The OpenAPI
//! surface (utoipa) is still open for a later phase.

mod admin;
mod auth;
mod background_refresh;
mod env_file;
mod media;
mod playback;
mod games;
mod notifications;
mod openapi;
mod metadata_enrichment;
mod provider_registry;
mod setup;
mod state;

use std::sync::Arc;

use axum::{
    Json, Router,
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use state::{AppState, ServerMode};

#[tokio::main]
async fn main() {
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

    let state = Arc::new(AppState {
        mode: RwLock::new(mode),
    });

    // Faza 5 completion's "Zadanie w tle": one detached long-running task,
    // started here so it can't delay the server's own startup or block a
    // request — see background_refresh.rs's own doc comment for exactly
    // what it does and how it stays safe around locked items/fields.
    tokio::spawn(background_refresh::run(state.clone()));
    tokio::spawn(notifications::run(state.clone()));
    tokio::spawn(async {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            lisek_playback::transcode::cleanup().await;
        }
    });

    // Reachable with no session at all: health/status checks, the setup
    // wizard (which by definition runs before any account exists), and
    // login itself.
    let public_routes = Router::new()
        .route("/api/health", get(health))
        .route("/api/openapi.json", get(openapi::document))
        .route("/api/playback/{session_id}/stream", get(playback::stream))
        .route("/api/playback/{session_id}/hls/{file}", get(playback::hls))
        .route("/api/status", get(setup::status))
        .route("/api/setup/database/test", post(setup::test_database))
        .route("/api/setup/finish", post(setup::finish))
        .route("/api/auth/login", post(auth::login));

    // PROTECTED ROUTES — every handler here takes `auth::AuthUser` as a
    // parameter; that extractor rejecting the request with 401 *is* the
    // enforcement (see auth.rs's doc comment on `AuthUser`). Add every
    // future data-serving endpoint (libraries, media, admin settings, ...)
    // to this group, not `public_routes`.
    let protected_routes = Router::new()
        .route("/api/auth/me", get(auth::me))
        .route("/api/admin/webhooks", get(notifications::list).post(notifications::create))
        .route("/api/admin/webhooks/{id}", delete(notifications::delete))
        .route("/api/admin/webhooks/deliveries", get(notifications::deliveries))
        .route("/api/games", get(games::list))
        .route("/api/admin/games", get(games::admin_list))
        .route("/api/admin/games/{key}", put(games::configure))
        .route("/api/games/{key}/scores", post(games::save))
        .route("/api/games/{key}/leaderboard", get(games::leaderboard))
        .route("/api/media/{item_id}/playback", post(playback::start))
        .route("/api/playback/{session_id}/progress", put(playback::progress))
        .route("/api/media/{item_id}/state", get(playback::media_state).put(playback::set_flags))
        .route("/api/me/list", get(playback::my_list))
        .route("/api/admin/stats/users", get(playback::stats))
        .route("/api/admin/playback/safety", get(playback::safety).put(playback::set_safety))
        .route("/api/auth/logout", post(auth::logout))
        // Admin-only in practice, not in routing: every handler in
        // admin.rs re-checks `user.is_admin` itself (see that module's doc
        // comment) rather than this router having a separate admin-only
        // sub-group, matching how AuthUser itself is the whole enforcement
        // for "protected" above.
        .route("/api/admin/providers", get(admin::list_providers))
        .route(
            "/api/admin/providers/{provider_key}",
            put(admin::update_provider),
        )
        .route(
            "/api/admin/providers/{provider_key}/key",
            put(admin::set_provider_key).delete(admin::clear_provider_key),
        )
        .route(
            "/api/admin/libraries",
            get(admin::list_libraries).post(admin::create_library),
        )
        .route(
            "/api/admin/libraries/{library_id}",
            delete(admin::delete_library),
        )
        .route(
            "/api/admin/libraries/{library_id}/reorder",
            put(admin::move_library),
        )
        .route(
            "/api/admin/libraries/{library_id}/scan",
            post(admin::scan_library),
        )
        .route(
            "/api/admin/libraries/{library_id}/items",
            get(admin::list_library_items),
        )
        .route(
            "/api/admin/libraries/{library_id}/provider-order/{item_type}",
            get(admin::get_provider_order).put(admin::set_provider_order),
        )
        .route("/api/admin/media/fallback", get(admin::list_fallback_items))
        .route(
            "/api/admin/media/{item_id}",
            get(admin::get_media_item).put(admin::update_media_item),
        )
        .route(
            "/api/admin/media/{item_id}/migrate-to-media-folder",
            post(admin::migrate_fallback_item),
        )
        // Not admin-only — see media.rs's own doc comment.
        .route("/api/home", get(media::home_feed))
        .route("/api/libraries", get(media::list_libraries))
        .route(
            "/api/libraries/{library_id}/items",
            get(media::library_items),
        )
        .route(
            "/api/libraries/{library_id}/recent",
            get(media::library_recent),
        )
        .route(
            "/api/libraries/{library_id}/recommended",
            get(media::library_recommended),
        )
        .route(
            "/api/libraries/{library_id}/popular",
            get(media::library_popular),
        )
        .route("/api/media/{item_id}", get(media::media_item_detail))
        .route(
            "/api/media/{item_id}/seasons",
            get(media::media_item_seasons),
        )
        .route("/api/media/{item_id}/poster", get(media::poster_image))
        .route("/api/media/{item_id}/backdrop", get(media::backdrop_image))
        .route("/api/media/{item_id}/logo", get(media::logo_image));

    // Permissive for now: same-origin in dev (via the frontend's Vite
    // proxy) and planned same-origin in production (one reverse proxy in
    // front of both). The custom Authorization header and the frontend's
    // session cookie both need this revisited before this server is ever
    // exposed to a different origin than its own web app.
    let app = public_routes
        .merge(protected_routes)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let bind_addr = std::env::var("LISEK_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .unwrap_or_else(|err| panic!("failed to bind to {bind_addr}: {err}"));

    tracing::info!(
        "LisekMovie backend listening on http://{}",
        listener.local_addr().unwrap()
    );

    axum::serve(listener, app).await.expect("server error");
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

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "lisek-server" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_reports_ok() {
        let Json(body) = health().await;
        assert_eq!(body["status"], "ok");
    }
}
