//! Base of the OpenAPI document. Paths are not listed here: `routes.rs`
//! registers each handler with `utoipa-axum`, which adds its
//! `#[utoipa::path]` to the document at the same time it adds the route.
use utoipa::Modify;
use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};

#[derive(OpenApi)]
#[openapi(
    info(title = "LisekMovie", version = "0.1.0"),
    modifiers(&SecurityAddon),
    tags(
        (name = "setup", description = "Health and the First-Run Wizard"),
        (name = "auth", description = "Sessions and passwords"),
        (name = "media", description = "Browsing the catalog"),
        (name = "admin", description = "Administration (administrators only)")
    )
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        openapi.components.get_or_insert_with(Default::default).add_security_scheme(
            "lisek_auth",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::with_description(
                "Authorization",
                "LisekMovie Client=\"web\", Device=\"Browser\", DeviceId=\"...\", Version=\"0.1.0\", Token=\"...\"",
            ))),
        );
    }
}
