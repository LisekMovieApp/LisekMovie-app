//! Downloads a poster/backdrop image from a metadata provider's CDN and
//! writes it to disk — either directly into a library's own media folder
//! (`poster.jpg`/`backdrop.jpg` next to the video
//! file — see [`save_image`]) or into the app's own local metadata cache,
//! so a served poster never depends on the video library's own folder
//! permissions (which may be read-only, or on removable/network storage)
//! and the app never has to re-fetch the same image from a provider on
//! every page load.
//!
//! Cache layout: `{config_dir}/metadata/posters/{item_id}.jpg`,
//! `{config_dir}/metadata/backdrops/{item_id}.jpg`, and
//! `{config_dir}/metadata/logos/{item_id}.png` — see
//! [`poster_cache_path`]/[`backdrop_cache_path`]/[`logo_cache_path`].
//! `config_dir` itself is
//! `lisek-server`'s concern (an env var with a sensible default); this
//! module only needs *a* directory, not where it comes from. Likewise, the
//! exact filename used inside a media folder (`poster.jpg`, `backdrop.jpg`)
//! is decided by the caller (`lisek-server`'s `metadata_enrichment`) — this
//! module only needs *a* destination path, not the naming convention.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ImageCacheError {
    #[error("failed to download image: {0}")]
    Download(String),
    #[error("provider returned {0} for the image request")]
    UnexpectedStatus(u16),
    #[error("failed to write cached image to disk: {0}")]
    Io(String),
}

impl From<reqwest::Error> for ImageCacheError {
    fn from(err: reqwest::Error) -> Self {
        ImageCacheError::Download(err.to_string())
    }
}

impl From<std::io::Error> for ImageCacheError {
    fn from(err: std::io::Error) -> Self {
        ImageCacheError::Io(err.to_string())
    }
}

/// Where a given item's cached poster lives (whether or not it's been
/// downloaded yet) — a pure path computation, so both the download step and
/// whatever later serves the file agree on the same layout without either
/// depending on the other.
pub fn poster_cache_path(config_dir: &Path, item_id: Uuid) -> PathBuf {
    config_dir
        .join("metadata")
        .join("posters")
        .join(format!("{item_id}.jpg"))
}

/// Same as [`poster_cache_path`] for the backdrop image.
pub fn backdrop_cache_path(config_dir: &Path, item_id: Uuid) -> PathBuf {
    config_dir
        .join("metadata")
        .join("backdrops")
        .join(format!("{item_id}.jpg"))
}

/// Same as [`poster_cache_path`] for the transparent title-logo image —
/// `.png`, not `.jpg`, since a logo needs real transparency and every logo
/// this app ever downloads has already been filtered down to PNG at the
/// provider layer (see `LisekMovie-plugin-tmdb`'s own doc comment on why a
/// non-PNG logo is treated as "no logo" rather than saved mislabeled).
pub fn logo_cache_path(config_dir: &Path, item_id: Uuid) -> PathBuf {
    config_dir
        .join("metadata")
        .join("logos")
        .join(format!("{item_id}.png"))
}

/// Downloads whatever's at `url` and writes it to `dest`, creating any
/// missing parent directories first. Always overwrites an existing file at
/// `dest` — a re-scan that finds a new provider match for an item is
/// expected to replace its previously cached image, not leave a stale one
/// behind.
pub async fn download_image(
    http: &reqwest::Client,
    url: &str,
    dest: &Path,
) -> Result<(), ImageCacheError> {
    let bytes = fetch_bytes(http, url).await?;
    write_bytes(dest, &bytes).await?;
    Ok(())
}

/// Downloads whatever's at `url` once, then writes it to `media_folder_dest`
/// when that's `Some` (a library with "save metadata in the media folder"
/// turned on) — falling back to `fallback_dest` (the local config-dir
/// cache) only when that write fails with a permission error, which is by
/// far the most common real-world reason a media folder can't be written
/// to (a read-only network share, a mount the app's own process doesn't
/// own). Any *other* write error (disk full, a genuinely broken path) is
/// still propagated rather than silently masked by a fallback attempt that
/// would likely fail the exact same way.
///
/// Returns `true` when the fallback path was the one actually used —
/// `false` covers both "no media-folder destination was requested at all"
/// (the library has the setting turned off) and "the media-folder write
/// succeeded." The caller (`lisek-server`'s `metadata_enrichment`) persists
/// that as `media_items.metadata_fallback`, but only means it as "a
/// permission problem happened" in the `Some(dest)` case — see this
/// crate's own doc comment and the Faza 5 round-2 migration for why a
/// deliberately-off setting is never treated as an exception worth
/// flagging.
pub async fn save_image(
    http: &reqwest::Client,
    url: &str,
    media_folder_dest: Option<&Path>,
    fallback_dest: &Path,
) -> Result<bool, ImageCacheError> {
    let bytes = fetch_bytes(http, url).await?;
    store_image(&bytes, media_folder_dest, fallback_dest).await
}

/// Downloads an image without writing it anywhere — for callers that write
/// the same bytes to several items (every episode of one series).
pub async fn fetch_image(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, ImageCacheError> {
    fetch_bytes(http, url).await
}

/// The write half of [`save_image`], same destinations and fallback rule,
/// same return value.
pub async fn store_image(
    bytes: &[u8],
    media_folder_dest: Option<&Path>,
    fallback_dest: &Path,
) -> Result<bool, ImageCacheError> {
    if let Some(dest) = media_folder_dest {
        let result = write_bytes(dest, bytes).await;
        if should_fall_back(&result) {
            let err = result.expect_err("should_fall_back only returns true for an Err");
            tracing::warn!(
                path = %dest.display(),
                %err,
                "media-folder image write denied — falling back to the local cache"
            );
        } else {
            return result.map(|()| false).map_err(Into::into);
        }
    }

    write_bytes(fallback_dest, bytes).await?;
    Ok(media_folder_dest.is_some())
}

async fn fetch_bytes(http: &reqwest::Client, url: &str) -> Result<Vec<u8>, ImageCacheError> {
    let response = http.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        return Err(ImageCacheError::UnexpectedStatus(status.as_u16()));
    }
    Ok(response.bytes().await?.to_vec())
}

async fn write_bytes(dest: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(dest, bytes).await
}

fn is_permission_denied(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::PermissionDenied
}

/// Whether a media-folder write's outcome should trigger the fallback to
/// the local cache — factored out on its own (rather than inlined into
/// [`save_image`]) so it can be unit-tested against a manually constructed
/// [`std::io::Error`] instead of a real permission-denied filesystem
/// condition. This module's own test suite runs as root in this project's
/// CI sandbox, and root bypasses ordinary Unix permission bits entirely —
/// a `chmod`-based integration test here would silently pass without ever
/// exercising the fallback branch it claims to, which is exactly the kind
/// of false-confidence bug this project has been burned by before (see the
/// project notes on measuring actual behavior instead of trusting
/// plausible-looking code). On the real Windows deployment target this
/// runs on, a denied ACL write reliably surfaces as `PermissionDenied`
/// regardless of this test-environment quirk.
fn should_fall_back(write_result: &std::io::Result<()>) -> bool {
    matches!(write_result, Err(err) if is_permission_denied(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn cache_paths_are_namespaced_by_kind_and_named_by_item_id() {
        let config_dir = Path::new("/config");
        let item_id = Uuid::nil();

        assert_eq!(
            poster_cache_path(config_dir, item_id),
            Path::new("/config/metadata/posters/00000000-0000-0000-0000-000000000000.jpg")
        );
        assert_eq!(
            backdrop_cache_path(config_dir, item_id),
            Path::new("/config/metadata/backdrops/00000000-0000-0000-0000-000000000000.jpg")
        );
        assert_eq!(
            logo_cache_path(config_dir, item_id),
            Path::new("/config/metadata/logos/00000000-0000-0000-0000-000000000000.png")
        );
    }

    #[tokio::test]
    async fn download_image_writes_the_response_body_to_disk() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/poster.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fake-jpeg-bytes".to_vec()))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("nested").join("poster.jpg");
        let http = reqwest::Client::new();

        download_image(&http, &format!("{}/poster.jpg", server.uri()), &dest)
            .await
            .unwrap();

        let written = std::fs::read(&dest).unwrap();
        assert_eq!(written, b"fake-jpeg-bytes");
    }

    #[tokio::test]
    async fn download_image_reports_a_non_success_status_instead_of_caching_an_error_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing.jpg"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("poster.jpg");
        let http = reqwest::Client::new();

        let result = download_image(&http, &format!("{}/missing.jpg", server.uri()), &dest).await;
        assert!(matches!(
            result,
            Err(ImageCacheError::UnexpectedStatus(404))
        ));
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn download_image_overwrites_an_existing_cached_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/poster.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"new-bytes".to_vec()))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("poster.jpg");
        std::fs::write(&dest, b"stale-bytes").unwrap();
        let http = reqwest::Client::new();

        download_image(&http, &format!("{}/poster.jpg", server.uri()), &dest)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"new-bytes");
    }

    #[test]
    fn should_fall_back_only_for_a_permission_denied_write_error() {
        assert!(should_fall_back(&Err(std::io::Error::from(
            ErrorKind::PermissionDenied
        ))));
        assert!(!should_fall_back(&Ok(())));
        assert!(!should_fall_back(&Err(std::io::Error::from(
            ErrorKind::NotFound
        ))));
        assert!(!should_fall_back(&Err(std::io::Error::from(
            ErrorKind::Other
        ))));
    }

    #[tokio::test]
    async fn save_image_writes_to_the_media_folder_and_never_touches_the_fallback_when_writable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/poster.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"media-folder-bytes".to_vec()))
            .mount(&server)
            .await;

        let media_dir = tempfile::tempdir().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let media_dest = media_dir.path().join("poster.jpg");
        let fallback_dest = cache_dir.path().join("posters").join("fallback.jpg");
        let http = reqwest::Client::new();

        let used_fallback = save_image(
            &http,
            &format!("{}/poster.jpg", server.uri()),
            Some(&media_dest),
            &fallback_dest,
        )
        .await
        .unwrap();

        assert!(!used_fallback);
        assert_eq!(std::fs::read(&media_dest).unwrap(), b"media-folder-bytes");
        assert!(!fallback_dest.exists());
    }

    #[tokio::test]
    async fn save_image_writes_straight_to_the_fallback_when_no_media_folder_destination_is_given()
    {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/backdrop.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"cache-only-bytes".to_vec()))
            .mount(&server)
            .await;

        let cache_dir = tempfile::tempdir().unwrap();
        let fallback_dest = cache_dir.path().join("backdrops").join("item.jpg");
        let http = reqwest::Client::new();

        let used_fallback = save_image(
            &http,
            &format!("{}/backdrop.jpg", server.uri()),
            None,
            &fallback_dest,
        )
        .await
        .unwrap();

        // `false` here specifically means "no media-folder destination was
        // ever requested" — this is the "the library has the setting
        // turned off" case, not a permission problem, so the caller must
        // not record this as a `metadata_fallback` exception either.
        assert!(!used_fallback);
        assert_eq!(std::fs::read(&fallback_dest).unwrap(), b"cache-only-bytes");
    }

    #[tokio::test]
    async fn save_image_propagates_a_non_permission_write_error_instead_of_silently_falling_back() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/poster.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"bytes".to_vec()))
            .mount(&server)
            .await;

        let media_dir = tempfile::tempdir().unwrap();
        // A regular file where a directory component is expected: this
        // isn't a permission problem, so it must never be treated as one —
        // `create_dir_all` fails with `NotADirectory`/`AlreadyExists`
        // rather than `PermissionDenied`, for any user including root
        // (unlike a `chmod`-based test, which root would simply bypass —
        // see `should_fall_back`'s doc comment).
        let blocking_file = media_dir.path().join("not-a-directory");
        std::fs::write(&blocking_file, b"i am a file, not a directory").unwrap();
        let media_dest = blocking_file.join("poster.jpg");

        let cache_dir = tempfile::tempdir().unwrap();
        let fallback_dest = cache_dir.path().join("posters").join("fallback.jpg");
        let http = reqwest::Client::new();

        let result = save_image(
            &http,
            &format!("{}/poster.jpg", server.uri()),
            Some(&media_dest),
            &fallback_dest,
        )
        .await;

        assert!(matches!(result, Err(ImageCacheError::Io(_))));
        assert!(
            !fallback_dest.exists(),
            "a non-permission error must not silently fall back to the cache"
        );
    }
}
