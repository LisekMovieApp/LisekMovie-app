//! Library CRUD and the shared navigation order.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use lisek_core::Library;
use lisek_library::LibraryType;

use crate::{DbError, PgPool};

/// The row shape a `libraries` SELECT comes back as.
type LibraryRow = (Uuid, String, String, i32, Vec<String>, bool, DateTime<Utc>);

fn library_from_row(row: LibraryRow) -> Library {
    Library {
        id: row.0,
        name: row.1,
        library_type: row.2,
        sort_order: row.3,
        root_paths: row.4,
        save_metadata_in_media_folder: row.5,
        created_at: row.6,
    }
}

/// Creates a new library, appended after every existing one in sort order.
/// `library_type` must be one of [`lisek_library::LibraryType`]'s canonical
/// DB strings (`"movies"` / `"tv_shows"`) — anything else comes back as
/// [`DbError::InvalidLibraryType`] before any query runs, so a typo from a
/// future caller can't silently create a library nothing can ever scan.
pub async fn create_library(
    pool: &PgPool,
    name: &str,
    library_type: &str,
    root_paths: &[String],
    save_metadata_in_media_folder: bool,
) -> Result<Library, DbError> {
    if LibraryType::from_db_str(library_type).is_none() {
        return Err(DbError::InvalidLibraryType(library_type.to_string()));
    }

    let next_sort_order: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order) + 1, 0) FROM libraries")
            .fetch_one(pool)
            .await?;

    let row = sqlx::query_as::<_, LibraryRow>(
        r#"
        INSERT INTO libraries (name, library_type, sort_order, root_paths, save_metadata_in_media_folder)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, name, library_type, sort_order, root_paths, save_metadata_in_media_folder, created_at
        "#,
    )
    .bind(name)
    .bind(library_type)
    .bind(next_sort_order)
    .bind(root_paths)
    .bind(save_metadata_in_media_folder)
    .fetch_one(pool)
    .await?;

    Ok(library_from_row(row))
}

/// Every configured library, in the admin's chosen display order.
pub async fn list_libraries(pool: &PgPool) -> Result<Vec<Library>, DbError> {
    let rows = sqlx::query_as::<_, LibraryRow>(
        r#"
        SELECT id, name, library_type, sort_order, root_paths, save_metadata_in_media_folder, created_at
        FROM libraries
        ORDER BY sort_order ASC, name ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(library_from_row).collect())
}

/// Looks up one library by id — used by the scan-trigger endpoint to read
/// its `root_paths` and `library_type` before walking the filesystem, and
/// its `save_metadata_in_media_folder` before enriching any item it finds.
pub async fn get_library(pool: &PgPool, library_id: Uuid) -> Result<Option<Library>, DbError> {
    let row = sqlx::query_as::<_, LibraryRow>(
        r#"
        SELECT id, name, library_type, sort_order, root_paths, save_metadata_in_media_folder, created_at
        FROM libraries
        WHERE id = $1
        "#,
    )
    .bind(library_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(library_from_row))
}

/// Deletes a library and, via `ON DELETE CASCADE`, every media item and
/// per-library provider override that belonged to it.
pub async fn delete_library(pool: &PgPool, library_id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query("DELETE FROM libraries WHERE id = $1")
        .bind(library_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::LibraryNotFound);
    }
    Ok(())
}

/// Which way to nudge a library in the admin's shared display order — see
/// [`move_library`]. A plain swap with the immediate neighbor (not a full
/// reorder-to-index) is all the Admin Panel's "W górę"/"W dół" button pair
/// (Runda 9) actually needs — no drag target to compute, no partial-list
/// ordering to reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryMoveDirection {
    Up,
    Down,
}

/// Swaps `library_id`'s `sort_order` with its immediate neighbor in that
/// direction — the Admin Panel's "W górę"/"W dół" buttons (Runda 9). This
/// `sort_order` column has actually been in the schema since migration
/// 0001 (see its own comment: "sort_order is set and reordered globally by
/// the admin, not per-user") and [`list_libraries`] already reads it — this
/// function is the first thing that ever *writes* to it after creation, and
/// [`lisek_server::media::list_libraries`]'s Navbar-facing endpoint is what
/// makes every signed-in account see the result identically, matching the
/// single shared ordering the schema comment always intended (not a
/// per-user preference).
///
/// A no-op (not an error) when `library_id` is already at that end of the
/// list — a real up/down button pair needs the boundary to be simply inert,
/// not a failure the frontend has to catch; disabling the button
/// client-side is a nicety on top, not something this function should have
/// to trust it happened. The two `UPDATE`s run inside one transaction so a
/// crash between them can never leave two libraries sharing a `sort_order`
/// — harmless for display (`list_libraries`'s `ORDER BY sort_order, name`
/// still resolves the tie) but not a state either query intends to produce.
pub async fn move_library(
    pool: &PgPool,
    library_id: Uuid,
    direction: LibraryMoveDirection,
) -> Result<(), DbError> {
    let rows: Vec<(Uuid, i32)> =
        sqlx::query_as("SELECT id, sort_order FROM libraries ORDER BY sort_order ASC, name ASC")
            .fetch_all(pool)
            .await?;

    let Some(pos) = rows.iter().position(|(id, _)| *id == library_id) else {
        return Err(DbError::LibraryNotFound);
    };

    let neighbor_pos = match direction {
        LibraryMoveDirection::Up => pos.checked_sub(1),
        LibraryMoveDirection::Down if pos + 1 < rows.len() => Some(pos + 1),
        LibraryMoveDirection::Down => None,
    };

    let Some(neighbor_pos) = neighbor_pos else {
        return Ok(());
    };

    let (this_id, this_order) = rows[pos];
    let (neighbor_id, neighbor_order) = rows[neighbor_pos];

    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE libraries SET sort_order = $1 WHERE id = $2")
        .bind(neighbor_order)
        .bind(this_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE libraries SET sort_order = $1 WHERE id = $2")
        .bind(this_order)
        .bind(neighbor_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(())
}
