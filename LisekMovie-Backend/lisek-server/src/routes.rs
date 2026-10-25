//! Every HTTP route, and the OpenAPI document generated from the same
//! registrations (`utoipa-axum`): a handler is reachable exactly when it is
//! documented, and its path comes from its own `#[utoipa::path]`.
//!
//! Three groups, by who may call them:
//! - `public`: no session (health, setup wizard, login, and the two
//!   ticket-authenticated playback streams);
//! - `user`: any signed-in account. A `SessionUser` layer guards the whole
//!   group; handlers then take `AuthUser` (full access) or `SessionUser`;
//! - `admin`: every `/api/admin/*` route. The `AdminUser` extractor runs as
//!   a `route_layer` in front of the whole group, so no handler in it can be
//!   reached by a non-administrator, whatever its own parameters say.

use std::sync::Arc;

use axum::Router;
use axum::http::header;
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use utoipa::OpenApi;
use utoipa::openapi::OpenApi as OpenApiDoc;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::{AdminUser, SessionUser};
use crate::openapi::ApiDoc;
use crate::state::SharedState;
use crate::{
    admin, auth, browser_notifications, collections, games, media, notifications, playback,
    scan_jobs, setup, telemetry, trickplay, users,
};

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
}

#[utoipa::path(get,path="/api/health",tag="setup",responses((status=200,body=HealthResponse)))]
pub async fn health() -> axum::Json<HealthResponse> {
    axum::Json(HealthResponse {
        status: "ok",
        service: "lisek-server",
    })
}

fn public() -> OpenApiRouter<SharedState> {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health))
        .routes(routes!(setup::status))
        .routes(routes!(setup::test_database))
        .routes(routes!(setup::finish))
        .routes(routes!(auth::login))
        // Authorized by the item-scoped playback ticket in the query
        // string, not a session: <video>, hls.js and <track> cannot send
        // headers.
        .routes(routes!(playback::stream))
        .routes(routes!(playback::hls))
        .routes(routes!(playback::subtitles))
        // Authenticates with its first WebSocket frame: browsers cannot set
        // headers on a WebSocket handshake.
        .routes(routes!(browser_notifications::socket))
}

fn user() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(routes!(auth::me))
        .routes(routes!(auth::logout))
        .routes(routes!(users::change))
        .routes(routes!(media::home_feed))
        .routes(routes!(media::list_libraries))
        .routes(routes!(media::library_items))
        .routes(routes!(media::library_recent))
        .routes(routes!(media::library_recommended))
        .routes(routes!(media::library_popular))
        .routes(routes!(media::media_item_detail))
        .routes(routes!(media::media_item_seasons))
        .routes(routes!(media::next_episode))
        .routes(routes!(collections::item_collection))
        .routes(routes!(collections::library_collections))
        .routes(routes!(collections::get_collection))
        .routes(routes!(collections::collection_poster))
        .routes(routes!(collections::collection_backdrop))
        .routes(routes!(trickplay::manifest))
        .routes(routes!(trickplay::sheet))
        .routes(routes!(media::poster_image))
        .routes(routes!(media::backdrop_image))
        .routes(routes!(media::logo_image))
        .routes(routes!(playback::start))
        .routes(routes!(playback::stop))
        .routes(routes!(telemetry::legacy_progress))
        .routes(routes!(telemetry::playback_progress))
        .routes(routes!(playback::media_state, playback::set_flags))
        .routes(routes!(playback::my_list))
        .routes(routes!(games::list))
        .routes(routes!(games::save))
        .routes(routes!(games::leaderboard))
}

fn admin() -> OpenApiRouter<SharedState> {
    OpenApiRouter::new()
        .routes(routes!(users::list, users::create))
        .routes(routes!(users::edit))
        .routes(routes!(users::reset))
        .routes(routes!(users::get_restrictions, users::set_restrictions))
        .routes(routes!(users::list_restrictions))
        .routes(routes!(admin::list_providers))
        .routes(routes!(admin::update_provider))
        .routes(routes!(admin::set_provider_key, admin::clear_provider_key))
        .routes(routes!(admin::list_libraries, admin::create_library))
        .routes(routes!(admin::delete_library))
        .routes(routes!(admin::move_library))
        .routes(routes!(admin::list_library_items))
        .routes(routes!(
            admin::get_provider_order,
            admin::set_provider_order
        ))
        .routes(routes!(admin::list_fallback_items))
        .routes(routes!(admin::get_media_item, admin::update_media_item))
        .routes(routes!(admin::migrate_fallback_item))
        .routes(routes!(scan_jobs::scan_library))
        .routes(routes!(scan_jobs::list))
        .routes(routes!(scan_jobs::get))
        .routes(routes!(notifications::list, notifications::create))
        .routes(routes!(notifications::delete))
        .routes(routes!(notifications::deliveries))
        .routes(routes!(games::admin_list))
        .routes(routes!(games::configure))
        .routes(routes!(playback::stats))
        .routes(routes!(telemetry::recent_sessions))
        .routes(routes!(trickplay::status))
        .routes(routes!(trickplay::regenerate))
        .routes(routes!(playback::settings, playback::set_settings))
        .routes(routes!(playback::capabilities))
        .routes(routes!(playback::safety, playback::set_safety))
}

/// Puts extractor `E` in front of every route of `group` as a route layer:
/// a request that fails `E` never reaches any handler of the group.
fn guarded<E>(
    group: OpenApiRouter<SharedState>,
    state: &SharedState,
) -> (Router<SharedState>, OpenApiDoc)
where
    E: axum::extract::FromRequestParts<SharedState> + Send + 'static,
{
    let (router, api) = group.split_for_parts();
    let router = router.route_layer(middleware::from_extractor_with_state::<E, SharedState>(
        state.clone(),
    ));
    (router, api)
}

/// The application router (state not yet attached) and its OpenAPI
/// document. `state` is only needed by the guard layers.
pub fn build(state: &SharedState) -> (Router<SharedState>, OpenApiDoc) {
    let (public_router, mut api) = public().split_for_parts();
    let (user_router, user_api) = guarded::<SessionUser>(user(), state);
    let (admin_router, admin_api) = guarded::<AdminUser>(admin(), state);
    api.merge(user_api);
    api.merge(admin_api);
    let router = public_router.merge(user_router);

    // Serialized once; the spec does not change while the process runs.
    let spec: Arc<str> = api.to_json().expect("OpenAPI document serializes").into();
    let router = router.merge(admin_router).route(
        "/api/openapi.json",
        get(move || {
            let spec = spec.clone();
            async move {
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    spec.to_string(),
                )
                    .into_response()
            }
        }),
    );
    (router, api)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AppState, ServerMode};
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    fn state() -> SharedState {
        let key = lisek_auth::secrets::SecretBox::generate_encoded_key();
        Arc::new(AppState::new(
            ServerMode::SetupRequired,
            lisek_auth::secrets::SecretBox::from_encoded_key(&key).unwrap(),
        ))
    }

    fn operations(api: &OpenApiDoc) -> Vec<(Method, String)> {
        let mut out = Vec::new();
        for (path, item) in &api.paths.paths {
            let concrete = path
                .split('/')
                .map(|s| {
                    if s.starts_with('{') {
                        "00000000-0000-0000-0000-000000000000"
                    } else {
                        s
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            for (method, op) in [
                (Method::GET, &item.get),
                (Method::POST, &item.post),
                (Method::PUT, &item.put),
                (Method::DELETE, &item.delete),
            ] {
                if op.is_some() {
                    out.push((method, concrete.clone()));
                }
            }
        }
        out
    }

    async fn send(app: &Router, method: Method, uri: &str, token: Option<&str>) -> StatusCode {
        let mut request = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("LisekMovie Token={token}"));
        }
        let mut request = request.body(Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from(([127, 0, 0, 1], 9))));
        app.clone().oneshot(request).await.unwrap().status()
    }

    #[test]
    fn admin_paths_live_only_in_the_guarded_group() {
        let (_, admin_api) = admin().split_for_parts();
        assert!(
            admin_api
                .paths
                .paths
                .keys()
                .all(|p| p.starts_with("/api/admin/"))
        );
        let (_, open_api) = public().merge(user()).split_for_parts();
        let leaked: Vec<_> = open_api
            .paths
            .paths
            .keys()
            .filter(|p| p.starts_with("/api/admin"))
            .collect();
        assert!(
            leaked.is_empty(),
            "admin paths outside the admin router: {leaked:?}"
        );
    }

    /// A handler with no auth parameter at all, to prove the layer alone
    /// protects the group.
    #[utoipa::path(get, path = "/api/admin/__probe", responses((status = 200)))]
    async fn unguarded_probe() -> &'static str {
        "reached"
    }

    #[tokio::test]
    async fn the_admin_layer_protects_handlers_that_take_no_extractor() {
        let state = state();
        let (router, _) = guarded::<AdminUser>(
            OpenApiRouter::new().routes(routes!(unguarded_probe)),
            &state,
        );
        let app = router.with_state(state);
        assert_eq!(
            send(&app, Method::GET, "/api/admin/__probe", None).await,
            StatusCode::UNAUTHORIZED
        );
        // With a token the guard still runs first (no database: 503).
        assert_eq!(
            send(&app, Method::GET, "/api/admin/__probe", Some("t")).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn previously_undocumented_endpoints_are_in_the_spec() {
        let state = state();
        let (_, api) = build(&state);
        let paths = &api.paths.paths;
        assert!(paths["/api/admin/libraries"].post.is_some());
        assert!(
            paths["/api/admin/libraries/{library_id}/provider-order/{item_type}"]
                .put
                .is_some()
        );
        assert!(paths["/api/admin/media/{item_id}"].put.is_some());
        assert!(
            paths["/api/admin/providers/{provider_key}/key"]
                .delete
                .is_some()
        );
        assert!(paths["/api/health"].get.is_some());
        let schemes = api.components.as_ref().unwrap().security_schemes.keys();
        assert!(schemes.into_iter().any(|k| k == "lisek_auth"));
    }

    #[tokio::test]
    async fn every_admin_operation_rejects_requests_without_a_session() {
        let state = state();
        let (router, api) = build(&state);
        let app = router.with_state(state);
        let admin_ops: Vec<_> = operations(&api)
            .into_iter()
            .filter(|(_, path)| path.starts_with("/api/admin/"))
            .collect();
        assert!(admin_ops.len() >= 30, "found {}", admin_ops.len());
        for (method, path) in admin_ops {
            let status = send(&app, method.clone(), &path, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
        }
    }

    #[tokio::test]
    async fn every_documented_operation_is_routed() {
        let state = state();
        let (router, api) = build(&state);
        let app = router.with_state(state);
        for (method, path) in operations(&api) {
            let status = send(&app, method.clone(), &path, Some("t")).await;
            assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {path}");
            assert_ne!(status, StatusCode::NOT_FOUND, "{method} {path}");
        }
        assert_eq!(
            send(&app, Method::GET, "/api/openapi.json", None).await,
            StatusCode::OK
        );
        assert_eq!(
            send(&app, Method::GET, "/api/health", None).await,
            StatusCode::OK
        );
    }
}
