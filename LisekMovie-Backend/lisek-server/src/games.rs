use axum::{Json,extract::{State,Path},http::StatusCode};
use serde::Deserialize;
use crate::{auth::{AdminUser,AuthUser},state::SharedState,playback::PlaybackError};
use lisek_db::games as db;

#[utoipa::path(get,path="/api/games",responses((status=200,body=Vec<db::Game>),(status=401)),security(("lisek_auth"=[])))]
pub async fn list(_auth:AuthUser,State(state):State<SharedState>)->Result<Json<Vec<db::Game>>,PlaybackError>{
    Ok(Json(db::list(&crate::playback::pool(&state).await?,false).await?))
}
#[utoipa::path(get,path="/api/admin/games",responses((status=200,body=Vec<db::Game>),(status=403)),security(("lisek_auth"=[])))]
pub async fn admin_list(_admin: AdminUser,State(state):State<SharedState>)->Result<Json<Vec<db::Game>>,PlaybackError>{
    Ok(Json(db::list(&crate::playback::pool(&state).await?,true).await?))
}
#[derive(Deserialize,utoipa::ToSchema)]
pub struct GameConfig {enabled:bool}
#[utoipa::path(put,path="/api/admin/games/{key}",params(("key"=String,Path)),request_body=GameConfig,responses((status=204),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn configure(_admin: AdminUser,State(state):State<SharedState>,Path(key):Path<String>,Json(req):Json<GameConfig>)->Result<StatusCode,PlaybackError>{
    if !db::set_enabled(&crate::playback::pool(&state).await?,&key,req.enabled).await?{return Err(PlaybackError(StatusCode::NOT_FOUND,"game_not_found"));}
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize,utoipa::ToSchema)]
pub struct ScoreRequest {score:i32}
#[utoipa::path(post,path="/api/games/{key}/scores",params(("key"=String,Path)),request_body=ScoreRequest,responses((status=204),(status=400),(status=404)),security(("lisek_auth"=[])))]
pub async fn save(auth:AuthUser,State(state):State<SharedState>,Path(key):Path<String>,Json(req):Json<ScoreRequest>)->Result<StatusCode,PlaybackError>{
    if !lisek_games::valid_score(&key,req.score){return Err(PlaybackError(StatusCode::BAD_REQUEST,"invalid_score"));}
    if !db::save_score(&crate::playback::pool(&state).await?,&key,auth.user.id,req.score).await?{return Err(PlaybackError(StatusCode::NOT_FOUND,"game_disabled"));}
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/games/{key}/leaderboard",params(("key"=String,Path)),responses((status=200,body=Vec<db::Score>),(status=404)),security(("lisek_auth"=[])))]
pub async fn leaderboard(_auth:AuthUser,State(state):State<SharedState>,Path(key):Path<String>)->Result<Json<Vec<db::Score>>,PlaybackError>{
    let pool=crate::playback::pool(&state).await?;
    if !db::enabled(&pool,&key).await?{return Err(PlaybackError(StatusCode::NOT_FOUND,"game_disabled"));}
    Ok(Json(db::leaderboard(&pool,&key).await?))
}
