use axum::{Json,extract::{State,Path},http::StatusCode};
use uuid::Uuid;
use serde_json::Value;
use crate::{auth::AdminUser,state::SharedState,playback::{PlaybackError,pool}};
use lisek_db::notifications as db;

#[utoipa::path(get,path="/api/admin/webhooks",responses((status=200,body=Vec<db::Endpoint>),(status=403)),security(("lisek_auth"=[])))]
pub async fn list(_admin: AdminUser,State(state):State<SharedState>)->Result<Json<Vec<db::Endpoint>>,PlaybackError>{
    Ok(Json(db::endpoints(&pool(&state).await?).await?))
}
#[utoipa::path(post,path="/api/admin/webhooks",request_body=db::EndpointInput,responses((status=200,body=Uuid),(status=400),(status=403)),security(("lisek_auth"=[])))]
pub async fn create(_admin: AdminUser,State(state):State<SharedState>,Json(req):Json<db::EndpointInput>)->Result<Json<Uuid>,PlaybackError>{
    if req.name.trim().is_empty() || req.name.len()>128 || req.url.len()>2048 || !matches!(req.destination_type.as_str(),"generic_json"|"discord"|"slack") || req.subscribed_events.iter().any(|e|e!="item_added"){
        return Err(PlaybackError(StatusCode::BAD_REQUEST,"invalid_webhook"));
    }
    lisek_notifications::validate_url(&req.url).map_err(|_|PlaybackError(StatusCode::BAD_REQUEST,"invalid_webhook_url"))?;
    Ok(Json(db::create(&pool(&state).await?,&req).await?))
}
#[utoipa::path(delete,path="/api/admin/webhooks/{id}",params(("id"=Uuid,Path)),responses((status=204),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn delete(_admin: AdminUser,State(state):State<SharedState>,Path(id):Path<Uuid>)->Result<StatusCode,PlaybackError>{
    if !db::delete(&pool(&state).await?,id).await?{return Err(PlaybackError(StatusCode::NOT_FOUND,"webhook_not_found"));}
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/admin/webhooks/deliveries",responses((status=200,body=Value),(status=403)),security(("lisek_auth"=[])))]
pub async fn deliveries(_admin: AdminUser,State(state):State<SharedState>)->Result<Json<Value>,PlaybackError>{
    Ok(Json(db::deliveries(&pool(&state).await?).await?))
}
pub async fn run(state:SharedState){
    let mut last_prune=std::time::Instant::now()-std::time::Duration::from_secs(3600);
    loop{
        if let Some(pool)=state.pool(){
            if last_prune.elapsed().as_secs()>=3600 { let _=db::prune_browser_events(&pool).await; last_prune=std::time::Instant::now(); }
            match db::claim(&pool).await{
                Ok(Some(delivery))=>{
                    let result=lisek_notifications::deliver(&delivery.url,&delivery.destination_type,&delivery.payload).await;
                    let (status,error)=match result{Ok(code)=>(Some(code),None),Err(code)=>(None,Some(code))};
                    let delay=lisek_notifications::retry_delay(delivery.attempt_count as u32);
                    if let Err(err)=db::complete(&pool,delivery.id,status,error,delivery.attempt_count,delay as i64).await{tracing::error!(%err,"cannot record webhook delivery");}
                    continue;
                }
                Ok(None)=>{},
                Err(err)=>tracing::error!(%err,"cannot read webhook queue"),
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}
