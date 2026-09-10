use axum::{Json, extract::{Path,State}, http::{StatusCode,header}, response::{IntoResponse,Response}};
use serde::{Deserialize,Serialize};
use uuid::Uuid;
use crate::{auth::{AuthUser,AuthReply,LoginResponse,no_store},state::SharedState,playback::pool};
use lisek_db::users as db;

pub struct UserError(StatusCode,&'static str);
impl IntoResponse for UserError { fn into_response(self)->Response { (self.0,Json(serde_json::json!({"error_code":self.1}))).into_response() } }
impl From<crate::playback::PlaybackError> for UserError { fn from(e:crate::playback::PlaybackError)->Self { Self(e.0,e.1) } }
impl From<lisek_db::DbError> for UserError { fn from(e:lisek_db::DbError)->Self {
    match e { lisek_db::DbError::UsernameTaken=>Self(StatusCode::CONFLICT,"username_taken"),e=>{tracing::error!(error=%e,"account operation failed");Self(StatusCode::INTERNAL_SERVER_ERROR,"internal_error")} }
} }
impl From<db::AccountError> for UserError { fn from(e:db::AccountError)->Self {match e {
    db::AccountError::Database(e)=>e.into(),db::AccountError::Forbidden=>Self(StatusCode::FORBIDDEN,"admin_required"),
    db::AccountError::NotFound=>Self(StatusCode::NOT_FOUND,"user_not_found"),db::AccountError::LastAdmin=>Self(StatusCode::CONFLICT,"last_administrator"),
    db::AccountError::Changed=>Self(StatusCode::CONFLICT,"credentials_changed"),
}} }
fn admin(auth:&AuthUser)->Result<(),UserError>{if auth.user.is_admin{Ok(())}else{Err(UserError(StatusCode::FORBIDDEN,"admin_required"))}}
fn validate(name:&str,language:&str)->Result<(),UserError>{
    if name.trim().is_empty() || name.len()>128 || name.chars().any(char::is_control) || !matches!(language,"pl"|"en") {return Err(UserError(StatusCode::BAD_REQUEST,"invalid_user"));} Ok(())
}
async fn hash(password:String)->Result<String,UserError>{
    tokio::task::spawn_blocking(move ||lisek_auth::hash_password(&password)).await.map_err(|_|UserError(StatusCode::INTERNAL_SERVER_ERROR,"internal_error"))?
        .map_err(|e|UserError(if matches!(e,lisek_auth::PasswordError::HashingFailed){StatusCode::INTERNAL_SERVER_ERROR}else{StatusCode::BAD_REQUEST},"invalid_password"))
}
#[derive(Deserialize,utoipa::ToSchema)]
pub struct CreateUser { username:String, #[serde(default)] is_admin:bool, ui_language:Option<String>, password:Option<String> }
#[derive(Deserialize,utoipa::ToSchema)]
pub struct EditUser { username:String, is_admin:bool, disabled:bool, ui_language:String }
#[derive(Deserialize,utoipa::ToSchema)]
pub struct ResetPassword { password:Option<String> }
#[derive(Serialize,utoipa::ToSchema)]
pub struct CreatedUser { user:lisek_core::User, temporary_password:String }
#[derive(Serialize,utoipa::ToSchema)]
pub struct TemporaryPassword { temporary_password:String }

#[utoipa::path(get,path="/api/admin/users",responses((status=200,body=Vec<lisek_core::User>),(status=403)),security(("lisek_auth"=[])))]
pub async fn list(auth:AuthUser,State(state):State<SharedState>)->Result<Json<Vec<lisek_core::User>>,UserError>{
    admin(&auth)?;Ok(Json(db::list(&pool(&state).await?).await?))
}
#[utoipa::path(post,path="/api/admin/users",request_body=CreateUser,responses((status=200,body=CreatedUser),(status=400),(status=403),(status=409)),security(("lisek_auth"=[])))]
pub async fn create(auth:AuthUser,State(state):State<SharedState>,Json(req):Json<CreateUser>)->Result<impl IntoResponse,UserError>{
    admin(&auth)?;
    let language=req.ui_language.unwrap_or(auth.user.ui_language.clone());
    validate(&req.username,&language)?;
    let password=req.password.unwrap_or_else(lisek_auth::generate_temporary_password);
    let hashed=hash(password.clone()).await?;
    let user=db::create(&pool(&state).await?,&auth.token_hash,req.username.trim(),&hashed,req.is_admin,&language).await?;
    Ok(([(header::CACHE_CONTROL,"no-store")],Json(CreatedUser{user,temporary_password:password})))
}
#[utoipa::path(put,path="/api/admin/users/{id}",params(("id"=Uuid,Path)),request_body=EditUser,responses((status=200,body=lisek_core::User),(status=400),(status=403),(status=404),(status=409)),security(("lisek_auth"=[])))]
pub async fn edit(auth:AuthUser,State(state):State<SharedState>,Path(id):Path<Uuid>,Json(req):Json<EditUser>)->Result<Json<lisek_core::User>,UserError>{
    admin(&auth)?;validate(&req.username,&req.ui_language)?;
    Ok(Json(db::edit(&pool(&state).await?,&auth.token_hash,id,req.username.trim(),req.is_admin,req.disabled,&req.ui_language).await?))
}
#[utoipa::path(post,path="/api/admin/users/{id}/password",params(("id"=Uuid,Path)),request_body=ResetPassword,responses((status=200,body=TemporaryPassword),(status=400),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn reset(auth:AuthUser,State(state):State<SharedState>,Path(id):Path<Uuid>,Json(req):Json<ResetPassword>)->Result<impl IntoResponse,UserError>{
    admin(&auth)?;
    let password=req.password.unwrap_or_else(lisek_auth::generate_temporary_password);
    let hashed=hash(password.clone()).await?;
    db::reset_password(&pool(&state).await?,&auth.token_hash,id,&hashed).await?;
    Ok(([(header::CACHE_CONTROL,"no-store")],Json(TemporaryPassword{temporary_password:password})))
}
#[derive(Deserialize,utoipa::ToSchema)]
pub struct ChangePassword { current_password:Option<String>, new_password:String }
#[utoipa::path(post,path="/api/auth/password",request_body=ChangePassword,responses((status=200,body=LoginResponse),(status=400),(status=401),(status=409)),security(("lisek_auth"=[])))]
pub async fn change(auth:AuthUser,State(state):State<SharedState>,Json(req):Json<ChangePassword>)->Result<AuthReply,UserError>{
    if req.new_password.len()>1024 || req.new_password.chars().count()<lisek_auth::MIN_PASSWORD_LENGTH || req.current_password.as_ref().is_some_and(|p|p.len()>1024){return Err(UserError(StatusCode::BAD_REQUEST,"invalid_password"));}
    let pool=pool(&state).await?;
    let old_hash=db::credential_hash(&pool,auth.user.id).await?.ok_or(UserError(StatusCode::UNAUTHORIZED,"invalid_token"))?;
    let previous=old_hash.clone();
    let force=auth.user.must_change_password;
    let checked=tokio::task::spawn_blocking(move ||{
        if !force && !req.current_password.as_ref().is_some_and(|p|lisek_auth::verify_password(p,&previous).unwrap_or(false)){return Err("incorrect_current_password");}
        if lisek_auth::verify_password(&req.new_password,&previous).unwrap_or(false){return Err("password_must_differ");}
        lisek_auth::hash_password(&req.new_password).map_err(|_|"internal_error")
    }).await.map_err(|_|UserError(StatusCode::INTERNAL_SERVER_ERROR,"internal_error"))?
        .map_err(|code|UserError(if code=="internal_error"{StatusCode::INTERNAL_SERVER_ERROR}else{StatusCode::BAD_REQUEST},code))?;
    let token=lisek_auth::generate_session_token();
    let user=db::change_password(&pool,auth.user.id,&auth.token_hash,&old_hash,&checked,&lisek_auth::hash_session_token(&token)).await?;
    Ok(no_store(LoginResponse{token,user}))
}
