//! Playback persistence. Every mutation is scoped to the authenticated owner.

#[derive(Debug,Serialize,serde::Deserialize,sqlx::FromRow,utoipa::ToSchema)]
pub struct SafetyConfig {
    pub enabled:bool,
    pub max_width:i32,
    pub max_height:i32,
    pub max_bitrate_kbps:Option<i32>,
}
pub async fn safety(pool:&PgPool)->Result<SafetyConfig,DbError>{
    Ok(sqlx::query_as("SELECT enabled,max_width,max_height,max_bitrate_kbps FROM transcode_safety_config WHERE id=true").fetch_one(pool).await?)
}
pub async fn set_safety(pool:&PgPool,config:&SafetyConfig)->Result<(),DbError>{
    sqlx::query("UPDATE transcode_safety_config SET enabled=$1,max_width=$2,max_height=$3,max_bitrate_kbps=$4 WHERE id=true")
        .bind(config.enabled).bind(config.max_width).bind(config.max_height).bind(config.max_bitrate_kbps).execute(pool).await?;
    Ok(())
}
pub async fn set_method(pool:&PgPool,id:Uuid,method:&str)->Result<(),DbError>{
    sqlx::query("UPDATE playback_sessions SET play_method=$2 WHERE id=$1").bind(id).bind(method).execute(pool).await?;Ok(())
}
pub async fn record_kill(pool:&PgPool,id:Uuid,reason:&str,width:i32,height:i32,bitrate:i32)->Result<(),DbError>{
    sqlx::query("INSERT INTO transcode_kills(session_id,item_id,user_id,reason,source_width,source_height,source_bitrate_kbps) SELECT id,item_id,user_id,$2,$3,$4,$5 FROM playback_sessions WHERE id=$1")
        .bind(id).bind(reason).bind(width).bind(height).bind(bitrate).execute(pool).await?;
    sqlx::query("UPDATE playback_sessions SET ended_at=now() WHERE id=$1").bind(id).execute(pool).await?;Ok(())
}
pub async fn end_session(pool:&PgPool,id:Uuid)->Result<(),DbError>{
    sqlx::query("UPDATE playback_sessions SET ended_at=now() WHERE id=$1").bind(id).execute(pool).await?;Ok(())
}
pub async fn session_alive(pool:&PgPool,id:Uuid)->Result<bool,DbError>{
    let (alive,):(bool,)=sqlx::query_as("SELECT EXISTS(SELECT 1 FROM playback_sessions p JOIN sessions s ON s.token_hash=p.auth_token_hash WHERE p.id=$1 AND p.ended_at IS NULL AND p.expires_at>now() AND s.expires_at>now())").bind(id).fetch_one(pool).await?;
    Ok(alive)
}
use crate::{DbError, PgPool};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct Source {
    pub id: Uuid,
    pub title: String,
    pub file_path: String,
    pub root_paths: Vec<String>,
}

pub async fn source(pool: &PgPool, item_id: Uuid) -> Result<Option<Source>, DbError> {
    Ok(sqlx::query_as("SELECT m.id, m.title, m.file_path, l.root_paths FROM media_items m JOIN libraries l ON l.id = m.library_id WHERE m.id = $1")
        .bind(item_id).fetch_optional(pool).await?)
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct MediaState {
    pub position_seconds: f64,
    pub duration_seconds: f64,
    pub watched: bool,
    pub favorite: bool,
}

pub async fn media_state(pool: &PgPool, user_id: Uuid, item_id: Uuid) -> Result<MediaState, DbError> {
    Ok(sqlx::query_as("SELECT position_seconds, duration_seconds, watched, favorite FROM user_media_state WHERE user_id = $1 AND item_id = $2")
        .bind(user_id).bind(item_id).fetch_optional(pool).await?
        .unwrap_or(MediaState { position_seconds: 0.0, duration_seconds: 0.0, watched: false, favorite: false }))
}

pub async fn set_flags(pool: &PgPool, user_id: Uuid, item_id: Uuid, watched: Option<bool>, favorite: Option<bool>) -> Result<(), DbError> {
    sqlx::query("INSERT INTO user_media_state (user_id, item_id, watched, favorite) VALUES ($1,$2,COALESCE($3,false),COALESCE($4,false)) ON CONFLICT (user_id,item_id) DO UPDATE SET watched=COALESCE($3,user_media_state.watched), position_seconds=CASE WHEN $3 IS NOT NULL THEN 0 ELSE user_media_state.position_seconds END, favorite=COALESCE($4,user_media_state.favorite), updated_at=now()")
        .bind(user_id).bind(item_id).bind(watched).bind(favorite).execute(pool).await?;
    Ok(())
}

pub async fn start(pool: &PgPool, user_id: Uuid, item_id: Uuid, auth_hash: &str, ticket_hash: &str) -> Result<Uuid, DbError> {
    let (id,): (Uuid,) = sqlx::query_as("INSERT INTO playback_sessions (user_id,item_id,auth_token_hash,ticket_hash,client_name,device_name,device_id) SELECT $1,$2,$3,$4,client,device,device_id FROM sessions WHERE token_hash=$3 AND user_id=$1 AND expires_at>now() RETURNING id")
        .bind(user_id).bind(item_id).bind(auth_hash).bind(ticket_hash).fetch_one(pool).await?;
    Ok(id)
}

pub async fn ticket_source(pool: &PgPool, id: Uuid, ticket_hash: &str) -> Result<Option<Source>, DbError> {
    Ok(sqlx::query_as("SELECT m.id,m.title,m.file_path,l.root_paths FROM playback_sessions p JOIN sessions s ON s.token_hash=p.auth_token_hash AND s.user_id=p.user_id JOIN media_items m ON m.id=p.item_id JOIN libraries l ON l.id=m.library_id WHERE p.id=$1 AND p.ticket_hash=$2 AND p.expires_at>now() AND p.ended_at IS NULL AND s.expires_at>now()")
        .bind(id).bind(ticket_hash).fetch_optional(pool).await?)
}

/// Server elapsed time counts watching; seeking never adds to viewing statistics.
pub async fn heartbeat(pool: &PgPool, user_id: Uuid, id: Uuid, position: f64, duration: f64, paused: bool, ended: bool) -> Result<bool, DbError> {
    let config = settings(pool).await?;
    let (position, completed) = config.resume_state(position, duration);
    let mut tx = pool.begin().await?;
    let row: Option<(Uuid,)> = sqlx::query_as("UPDATE playback_sessions SET watched_seconds=watched_seconds + CASE WHEN paused THEN 0 ELSE LEAST(30.0,GREATEST(0.0,EXTRACT(EPOCH FROM now()-last_heartbeat_at))) END, last_heartbeat_at=now(), paused=$3, ended_at=CASE WHEN $4 THEN now() ELSE NULL END WHERE id=$1 AND user_id=$2 AND ended_at IS NULL AND expires_at>now() RETURNING item_id")
        .bind(id).bind(user_id).bind(paused).bind(ended).fetch_optional(&mut *tx).await?;
    let Some((item_id,)) = row else { return Ok(false); };
    sqlx::query("INSERT INTO user_media_state (user_id,item_id,position_seconds,duration_seconds,watched) VALUES ($1,$2,$3,$4,$5) ON CONFLICT(user_id,item_id) DO UPDATE SET position_seconds=$3,duration_seconds=$4,watched=user_media_state.watched OR $5,updated_at=now()")
        .bind(user_id).bind(item_id).bind(position).bind(duration)
        .bind(completed)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct ListEntry {
    pub id: Uuid,
    pub title: String,
    pub item_type: String,
    pub position_seconds: f64,
    pub duration_seconds: f64,
    pub watched: bool,
    pub favorite: bool,
}

pub async fn my_list(pool: &PgPool, user_id: Uuid) -> Result<Vec<ListEntry>, DbError> {
    Ok(sqlx::query_as("SELECT m.id,m.title,m.item_type,u.position_seconds,u.duration_seconds,u.watched,u.favorite FROM user_media_state u JOIN media_items m ON m.id=u.item_id WHERE u.user_id=$1 ORDER BY u.updated_at DESC LIMIT 500")
        .bind(user_id).fetch_all(pool).await?)
}

#[derive(Serialize, sqlx::FromRow)]
pub struct UserStats {
    pub username: String,
    pub plays: i64,
    pub watched_seconds: f64,
}

pub async fn stats(pool: &PgPool) -> Result<Vec<UserStats>, DbError> {
    Ok(sqlx::query_as("SELECT u.username,COUNT(*) AS plays,SUM(p.watched_seconds)::float8 AS watched_seconds FROM playback_sessions p JOIN users u ON u.id=p.user_id WHERE p.watched_seconds>=20 GROUP BY u.id,u.username ORDER BY watched_seconds DESC")
        .fetch_all(pool).await?)
}


pub async fn settings(pool: &PgPool) -> Result<lisek_core::playback_settings::PlaybackSettings, DbError> {
    let (sqlx::types::Json(config),): (sqlx::types::Json<lisek_core::playback_settings::PlaybackSettings>,) = sqlx::query_as("SELECT settings FROM playback_settings WHERE id=true").fetch_one(pool).await?;
    Ok(config)
}
pub async fn set_settings(pool: &PgPool, settings: &lisek_core::playback_settings::PlaybackSettings) -> Result<(), DbError> {
    sqlx::query("UPDATE playback_settings SET settings=$1 WHERE id=true").bind(sqlx::types::Json(settings)).execute(pool).await?;
    Ok(())
}


pub async fn end_owned_session(pool:&PgPool,user_id:Uuid,id:Uuid)->Result<bool,DbError>{
    let result=sqlx::query("UPDATE playback_sessions SET ended_at=COALESCE(ended_at,now()) WHERE id=$1 AND user_id=$2").bind(id).bind(user_id).execute(pool).await?;
    Ok(result.rows_affected()>0)
}
