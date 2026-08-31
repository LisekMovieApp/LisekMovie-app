//! LisekMovie server binary.
//!
//! Faza 1 scope: prove the workspace links and boots end-to-end. Exposes a single
//! health-check endpoint. Real routing, middleware, and the OpenAPI surface
//! (utoipa) land in Faza 3 per the architecture plan.

use axum::{Json, Router, routing::get};
use serde_json::{Value, json};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new().route("/api/health", get(health));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("failed to bind to 0.0.0.0:8080");

    tracing::info!(
        "lisek-server listening on {}",
        listener.local_addr().unwrap()
    );

    axum::serve(listener, app).await.expect("server error");
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
