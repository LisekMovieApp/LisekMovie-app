//! Direct playback uses a revocable, item-scoped ticket for native browser requests.

async fn enforce_safety(pool:&lisek_db::PgPool,id:Uuid,info:&lisek_playback::transcode::SourceInfo)->Result<(),PlaybackError>{
    let config=db::safety(pool).await?;
    let policy=lisek_playback::TranscodeSafety{enabled:config.enabled,max_width:config.max_width as u32,max_height:config.max_height as u32,max_bitrate_kbps:config.max_bitrate_kbps.map(|v|v as u32)};
    if let Some(reason)=policy.rejection(info.width,info.height,info.bitrate_kbps){
        db::record_kill(pool,id,reason,info.width.min(i32::MAX as u32)as i32,info.height.min(i32::MAX as u32)as i32,info.bitrate_kbps.min(i32::MAX as u32)as i32).await?;
        return Err(PlaybackError(StatusCode::FORBIDDEN,"transcode_limit_exceeded"));
    }
    Ok(())
}
#[utoipa::path(get,path="/api/playback/{session_id}/hls/{file}",params(("session_id"=Uuid,Path),("file"=String,Path),("ticket"=String,Query)),responses((status=200,description="HLS manifest or segment"),(status=401),(status=404),(status=503)))]
pub async fn hls(State(state):State<SharedState>,Path((id,file)):Path<(Uuid,String)>,Query(query):Query<TicketQuery>)->Result<Response,PlaybackError>{
    let pool=pool(&state).await?;
    if !lisek_playback::transcode::valid_file(&file){return Err(PlaybackError(StatusCode::NOT_FOUND,"segment_not_found"));}
    if db::ticket_source(&pool,id,&lisek_auth::hash_session_token(&query.ticket)).await?.is_none(){return Err(PlaybackError(StatusCode::UNAUTHORIZED,"invalid_playback_ticket"));}
    if !lisek_playback::transcode::touch(&id.to_string()).await{return Err(PlaybackError(StatusCode::NOT_FOUND,"playback_not_found"));}
    let path=crate::state::config_dir().join("transcodes").join(id.to_string()).join(&file);
    if file=="index.m3u8"{
        for _ in 0..80 {
            if tokio::fs::try_exists(&path).await.unwrap_or(false){break;}
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
    let body=tokio::fs::read(&path).await.map_err(|_|PlaybackError(StatusCode::SERVICE_UNAVAILABLE,"segment_not_ready"))?;
    let (body,content_type)=if file=="index.m3u8"{
        let manifest=String::from_utf8(body).map_err(|_|PlaybackError(StatusCode::INTERNAL_SERVER_ERROR,"invalid_manifest"))?;
        let manifest=manifest.lines().map(|line|if line.starts_with('#')||line.is_empty(){line.to_owned()}else{format!("{line}?ticket={}",query.ticket)}).collect::<Vec<_>>().join("\n")+"\n";
        (manifest.into_bytes(),"application/vnd.apple.mpegurl")
    }else{(body,"video/mp2t")};
    Ok(([(header::CONTENT_TYPE,content_type),(header::CACHE_CONTROL,"private, no-store")],body).into_response())
}
#[utoipa::path(get,path="/api/admin/playback/safety",responses((status=200,body=db::SafetyConfig),(status=403)),security(("lisek_auth"=[])))]
pub async fn safety(auth:AuthUser,State(state):State<SharedState>)->Result<Json<db::SafetyConfig>,PlaybackError>{
    if !auth.user.is_admin{return Err(PlaybackError(StatusCode::FORBIDDEN,"admin_required"));}
    Ok(Json(db::safety(&pool(&state).await?).await?))
}
#[utoipa::path(put,path="/api/admin/playback/safety",request_body=db::SafetyConfig,responses((status=204),(status=400),(status=403)),security(("lisek_auth"=[])))]
pub async fn set_safety(auth:AuthUser,State(state):State<SharedState>,Json(req):Json<db::SafetyConfig>)->Result<StatusCode,PlaybackError>{
    if !auth.user.is_admin{return Err(PlaybackError(StatusCode::FORBIDDEN,"admin_required"));}
    if !(1..=16384).contains(&req.max_width)||!(1..=16384).contains(&req.max_height)||req.max_bitrate_kbps.is_some_and(|b|b<=0){return Err(PlaybackError(StatusCode::BAD_REQUEST,"invalid_safety_limits"));}
    db::set_safety(&pool(&state).await?,&req).await?;
    Ok(StatusCode::NO_CONTENT)
}
use axum::{Json, extract::{Path, Query, State}, http::{HeaderMap, StatusCode, header}, body::Body, response::{IntoResponse, Response}};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;
use crate::{auth::AuthUser, state::SharedState};
use lisek_db::playback as db;

pub struct PlaybackError(pub(crate) StatusCode, pub(crate) &'static str);
impl IntoResponse for PlaybackError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error_code": self.1, "message": "The requested playback action could not be completed."}))).into_response()
    }
}
impl From<lisek_db::DbError> for PlaybackError {
    fn from(err: lisek_db::DbError) -> Self {
        tracing::error!(error = %err, "playback database operation failed");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    }
}
pub(crate) async fn pool(state: &SharedState) -> Result<lisek_db::PgPool, PlaybackError> {
    state.pool().await.ok_or(PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}
async fn checked_file(source: &db::Source) -> Result<std::path::PathBuf, PlaybackError> {
    let missing = || PlaybackError(StatusCode::NOT_FOUND, "media_file_unavailable");
    let path = tokio::fs::canonicalize(&source.file_path).await.map_err(|_| missing())?;
    for root in &source.root_paths {
        if let Ok(root) = tokio::fs::canonicalize(root).await {
            if path.starts_with(root) && tokio::fs::metadata(&path).await.map(|m| m.is_file()).unwrap_or(false) {
                return Ok(path);
            }
        }
    }
    Err(missing())
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct StartResponse {
    pub session_id: Uuid,
    pub item_id: Uuid,
    pub title: String,
    pub stream_url: String,
    pub position_seconds: f64,
    pub time_offset_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub mode: String,
}
#[derive(Deserialize)]
pub struct StartOptions { #[serde(default)] transcode: bool }
#[utoipa::path(post, path="/api/media/{item_id}/playback", params(("item_id"=Uuid,Path)), responses((status=200,body=StartResponse),(status=401),(status=404)), security(("lisek_auth"=[])))]
pub async fn start(auth: AuthUser, State(state): State<SharedState>, Path(item_id): Path<Uuid>, Query(options): Query<StartOptions>) -> Result<Json<StartResponse>, PlaybackError> {
    let pool = pool(&state).await?;
    let source = db::source(&pool, item_id).await?.ok_or(PlaybackError(StatusCode::NOT_FOUND,"media_not_found"))?;
    let path = checked_file(&source).await?;
    let saved = db::media_state(&pool, auth.user.id, item_id).await?;
    let ticket = lisek_auth::generate_session_token();
    let session_id = db::start(&pool,auth.user.id,item_id,&auth.token_hash,&lisek_auth::hash_session_token(&ticket)).await?;
    let position = if saved.watched { 0.0 } else { saved.position_seconds };
    let mut response = StartResponse { session_id, item_id, title: source.title,
        stream_url: format!("/api/playback/{session_id}/stream?ticket={ticket}"),
        position_seconds: position, time_offset_seconds: 0.0, duration_seconds: None, mode: "direct_play".into() };
    if options.transcode {
        let info = match lisek_playback::transcode::probe(&path).await {
            Ok(info) => info,
            Err(reason) => { db::end_session(&pool,session_id).await?; tracing::warn!(%reason,"cannot probe video"); return Err(PlaybackError(StatusCode::SERVICE_UNAVAILABLE,"transcode_unavailable")); }
        };
        enforce_safety(&pool,session_id,&info).await?;
        let offset = position.min((info.duration-1.0).max(0.0));
        let directory = crate::state::config_dir().join("transcodes").join(session_id.to_string());
        if let Err(reason)=lisek_playback::transcode::start(&session_id.to_string(),&path,&directory,offset).await {
            db::end_session(&pool,session_id).await?;
            tracing::warn!(%reason,"cannot start transcoding");
            return Err(PlaybackError(StatusCode::SERVICE_UNAVAILABLE,"transcode_unavailable"));
        }
        db::set_method(&pool,session_id,"transcode").await?;
        response.mode="transcode".into();
        response.time_offset_seconds=offset;
        response.position_seconds=0.0;
        response.duration_seconds=Some(info.duration);
        response.stream_url=format!("/api/playback/{session_id}/hls/index.m3u8?ticket={ticket}");
        let guard_state=state.clone();
        let guard_info=info.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                let Some(pool)=guard_state.pool().await else { break; };
                if !db::session_alive(&pool,session_id).await.unwrap_or(false) || enforce_safety(&pool,session_id,&guard_info).await.is_err(){break;}
                if !lisek_playback::transcode::exists(&session_id.to_string()).await {break;}
            }
            lisek_playback::transcode::stop(&session_id.to_string()).await;
        });
    }
    Ok(Json(response))
}

#[derive(Deserialize)]
pub struct TicketQuery { ticket: String }

#[utoipa::path(get, path="/api/playback/{session_id}/stream", params(("session_id"=Uuid,Path),("ticket"=String,Query)), responses((status=200,description="Video bytes"),(status=206,description="Partial video bytes"),(status=401),(status=416)))]
pub async fn stream(State(state): State<SharedState>, Path(id): Path<Uuid>, Query(query): Query<TicketQuery>, headers: HeaderMap) -> Result<Response, PlaybackError> {
    let pool = pool(&state).await?;
    let source = db::ticket_source(&pool,id,&lisek_auth::hash_session_token(&query.ticket)).await?
        .ok_or(PlaybackError(StatusCode::UNAUTHORIZED,"invalid_playback_ticket"))?;
    let path = checked_file(&source).await?;
    let mut file = tokio::fs::File::open(&path).await.map_err(|_| PlaybackError(StatusCode::NOT_FOUND,"media_file_unavailable"))?;
    let length = file.metadata().await.map_err(|_| PlaybackError(StatusCode::INTERNAL_SERVER_ERROR,"media_read_failed"))?.len();
    let range = match headers.get(header::RANGE) {
        Some(value) => match value.to_str().ok().and_then(|value| lisek_playback::parse_range(value,length)) {
            Some(range) => Some(range),
            None => return Ok((StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE,format!("bytes */{length}"))]).into_response()),
        },
        None => None,
    };
    let (start, count) = range.map(|range| (range.start,range.end-range.start+1)).unwrap_or((0,length));
    file.seek(std::io::SeekFrom::Start(start)).await.map_err(|_| PlaybackError(StatusCode::INTERNAL_SERVER_ERROR,"media_read_failed"))?;
    let mut builder = Response::builder()
        .status(if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE,lisek_playback::content_type(&path))
        .header(header::CONTENT_LENGTH,count.to_string())
        .header(header::ACCEPT_RANGES,"bytes")
        .header(header::CACHE_CONTROL,"private, no-store")
        .header("Referrer-Policy","no-referrer")
        .header("X-Content-Type-Options","nosniff");
    if let Some(range) = range { builder = builder.header(header::CONTENT_RANGE,format!("bytes {}-{}/{length}",range.start,range.end)); }
    Ok(builder.body(Body::from_stream(ReaderStream::new(file.take(count)))).expect("static response headers"))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ProgressRequest {
    position_seconds: f64,
    duration_seconds: f64,
    paused: bool,
    #[serde(default)]
    ended: bool,
}
#[utoipa::path(put,path="/api/playback/{session_id}/progress",params(("session_id"=Uuid,Path)),request_body=ProgressRequest,responses((status=204),(status=400),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn progress(auth: AuthUser,State(state):State<SharedState>,Path(id):Path<Uuid>,Json(req):Json<ProgressRequest>) -> Result<StatusCode,PlaybackError> {
    if !req.position_seconds.is_finite() || !req.duration_seconds.is_finite() || req.position_seconds<0.0 || req.duration_seconds<0.0 || req.duration_seconds>604800.0 || req.position_seconds>req.duration_seconds+1.0 {
        return Err(PlaybackError(StatusCode::BAD_REQUEST,"invalid_progress"));
    }
    if !db::heartbeat(&pool(&state).await?,auth.user.id,id,req.position_seconds,req.duration_seconds,req.paused,req.ended).await? {
        return Err(PlaybackError(StatusCode::NOT_FOUND,"playback_not_found"));
    }
    if req.ended { lisek_playback::transcode::stop(&id.to_string()).await; }
    else { lisek_playback::transcode::touch(&id.to_string()).await; }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct FlagsRequest { watched: Option<bool>, favorite: Option<bool> }
#[utoipa::path(get,path="/api/media/{item_id}/state",params(("item_id"=Uuid,Path)),responses((status=200,body=Value),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn media_state(auth:AuthUser,State(state):State<SharedState>,Path(id):Path<Uuid>) -> Result<Json<Value>,PlaybackError> {
    let pool=pool(&state).await?;
    if db::source(&pool,id).await?.is_none() { return Err(PlaybackError(StatusCode::NOT_FOUND,"media_not_found")); }
    Ok(Json(json!(db::media_state(&pool,auth.user.id,id).await?)))
}
#[utoipa::path(put,path="/api/media/{item_id}/state",params(("item_id"=Uuid,Path)),request_body=FlagsRequest,responses((status=204),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn set_flags(auth:AuthUser,State(state):State<SharedState>,Path(id):Path<Uuid>,Json(req):Json<FlagsRequest>) -> Result<StatusCode,PlaybackError> {
    let pool=pool(&state).await?;
    if db::source(&pool,id).await?.is_none() { return Err(PlaybackError(StatusCode::NOT_FOUND,"media_not_found")); }
    db::set_flags(&pool,auth.user.id,id,req.watched,req.favorite).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/me/list",responses((status=200,body=Value),(status=401)),security(("lisek_auth"=[])))]
pub async fn my_list(auth:AuthUser,State(state):State<SharedState>) -> Result<Json<Value>,PlaybackError> {
    Ok(Json(json!(db::my_list(&pool(&state).await?,auth.user.id).await?)))
}
#[utoipa::path(get,path="/api/admin/stats/users",responses((status=200,body=Value),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn stats(auth:AuthUser,State(state):State<SharedState>) -> Result<Json<Value>,PlaybackError> {
    if !auth.user.is_admin { return Err(PlaybackError(StatusCode::FORBIDDEN,"admin_required")); }
    Ok(Json(json!(db::stats(&pool(&state).await?).await?)))
}
