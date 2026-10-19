//! Machine-readable API contract.
use axum::Json;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};

#[derive(OpenApi)]
#[openapi(
    info(title="LisekMovie", version="0.1.0"),
    paths(crate::users::list,crate::users::create,crate::users::edit,crate::users::reset,crate::users::change,crate::playback::stop,crate::browser_notifications::socket,crate::playback::settings,crate::playback::set_settings,crate::playback::capabilities,crate::notifications::list,crate::notifications::create,crate::notifications::delete,crate::notifications::deliveries,crate::playback::hls,crate::playback::safety,crate::playback::set_safety,crate::games::list,crate::games::admin_list,crate::games::configure,crate::games::save,crate::games::leaderboard,crate::setup::status,crate::setup::test_database,crate::setup::finish,crate::auth::login,crate::auth::me,crate::auth::logout,crate::admin::list_providers,crate::admin::update_provider,crate::admin::set_provider_key,crate::admin::list_libraries,crate::admin::delete_library,crate::admin::move_library,crate::scan_jobs::scan_library,crate::scan_jobs::list,crate::scan_jobs::get,crate::admin::list_library_items,crate::admin::get_provider_order,crate::admin::list_fallback_items,crate::admin::get_media_item,crate::admin::migrate_fallback_item,crate::media::home_feed,crate::media::list_libraries,crate::media::library_items,crate::media::library_recent,crate::media::library_recommended,crate::media::library_popular,crate::media::media_item_detail,crate::media::media_item_seasons,crate::media::poster_image,crate::media::backdrop_image,crate::media::logo_image,crate::playback::start,crate::playback::stream,crate::playback::progress,
        crate::playback::media_state,crate::playback::set_flags,crate::playback::my_list,crate::playback::stats)
)]
pub struct ApiDoc;

pub async fn document() -> Json<utoipa::openapi::OpenApi> {
    let mut doc=ApiDoc::openapi();
    doc.components.get_or_insert_with(Default::default).add_security_scheme(
        "lisek_auth",
        SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
            "Authorization", "LisekMovie Client=\"web\", Device=\"Browser\", DeviceId=\"...\", Version=\"0.1.0\", Token=\"...\""
        )))
    );
    Json(doc)
}


