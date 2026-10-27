//! Revalidated caching for images served under a stable URL (posters,
//! backdrops, logos, stills): `Cache-Control: private, no-cache` plus a
//! content `ETag`, and [`not_modified`] answering a matching
//! `If-None-Match` with `304 Not Modified` and no body. A browser keeps
//! the picture, asks again on every view, and gets the new one right after
//! a Fix match instead of an hour later.

use std::hash::{DefaultHasher, Hash, Hasher};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;

pub const REVALIDATE: &str = "private, no-cache";

/// A strong ETag from the content (length + SipHash).
pub fn etag(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("\"{:x}-{:016x}\"", bytes.len(), hasher.finish())
}

/// `If-None-Match: "a", W/"b"` contains `etag` (or is `*`).
fn matches(if_none_match: &HeaderValue, etag: &HeaderValue) -> bool {
    let (Ok(list), Ok(etag)) = (if_none_match.to_str(), etag.to_str()) else {
        return false;
    };
    let etag = etag.trim_start_matches("W/");
    list.split(',')
        .map(|tag| tag.trim().trim_start_matches("W/"))
        .any(|tag| tag == "*" || tag == etag)
}

/// Middleware: a `GET`/`HEAD` response with an `ETag` the client already
/// has becomes `304 Not Modified`.
pub async fn not_modified(request: Request, next: Next) -> Response {
    let conditional = matches!(request.method().as_str(), "GET" | "HEAD")
        .then(|| request.headers().get(header::IF_NONE_MATCH).cloned())
        .flatten();
    let response = next.run(request).await;
    let Some(if_none_match) = conditional else {
        return response;
    };
    if response.status() != StatusCode::OK {
        return response;
    }
    let Some(etag) = response.headers().get(header::ETAG).cloned() else {
        return response;
    };
    if !matches(&if_none_match, &etag) {
        return response;
    }
    let mut not_modified = Response::new(Body::empty());
    *not_modified.status_mut() = StatusCode::NOT_MODIFIED;
    let headers = not_modified.headers_mut();
    headers.insert(header::ETAG, etag);
    if let Some(cache_control) = response.headers().get(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, cache_control.clone());
    }
    not_modified
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;
    use tower::ServiceExt;

    #[test]
    fn etags_follow_the_content() {
        assert_eq!(etag(b"abc"), etag(b"abc"));
        assert_ne!(etag(b"abc"), etag(b"abd"));
        let tag = HeaderValue::from_str(&etag(b"abc")).unwrap();
        let list = HeaderValue::from_str(&format!("\"x\", W/{}", etag(b"abc"))).unwrap();
        assert!(matches(&list, &tag));
        assert!(matches(&HeaderValue::from_static("*"), &tag));
        assert!(!matches(&HeaderValue::from_static("\"other\""), &tag));
    }

    #[tokio::test]
    async fn a_known_etag_gets_304_without_a_body() {
        let app = Router::new()
            .route("/img", get(|| async { crate::media::image_response(b"jpeg".to_vec(), "image/jpeg") }))
            .layer(axum::middleware::from_fn(not_modified));
        let first = app
            .clone()
            .oneshot(Request::builder().uri("/img").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CACHE_CONTROL], REVALIDATE);
        let tag = first.headers()[header::ETAG].clone();
        let again = app
            .oneshot(
                Request::builder()
                    .uri("/img")
                    .header(header::IF_NONE_MATCH, tag.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(again.headers()[header::ETAG], tag);
        let body = axum::body::to_bytes(again.into_body(), 64).await.unwrap();
        assert!(body.is_empty());
    }
}
