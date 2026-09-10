//! Live browser notifications. Authentication is sent in the first frame, never in a URL.
use crate::state::SharedState;
use axum::{
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::Response,
};
use std::{sync::Arc, sync::LazyLock, time::Duration};
use tokio::sync::Semaphore;

static CONNECTIONS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(128)));

#[utoipa::path(get,path="/api/notifications/socket",responses((status=101,description="WebSocket. First frame: JSON {token:string}. Server sends ready then library_changed with items. Requires an active login; closes when revoked."),(status=503)))]
pub async fn socket(
    State(state): State<SharedState>,
    ws: WebSocketUpgrade,
) -> Result<Response, StatusCode> {
    let permit = CONNECTIONS
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(ws
        .max_message_size(4096)
        .max_frame_size(4096)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            run(socket, state).await;
        }))
}
async fn run(mut socket: WebSocket, state: SharedState) {
    let Some(pool) = state.pool().await else {
        return;
    };
    let Ok(Some(Ok(Message::Text(text)))) =
        tokio::time::timeout(Duration::from_secs(5), socket.recv()).await
    else {
        return;
    };
    #[derive(serde::Deserialize)]
    struct Login {
        token: String,
    }
    let Ok(login) = serde_json::from_str::<Login>(&text) else {
        return;
    };
    if login.token.is_empty() || login.token.len() > 1024 {
        return;
    }
    let hash = lisek_auth::hash_session_token(&login.token);
    if !matches!(
        lisek_db::find_user_by_session_token_hash(&pool, &hash).await,
        Ok(Some(lisek_core::User { must_change_password:false, .. }))
    ) {
        return;
    }
    let started = chrono::Utc::now();
    // Overlapping reads and a set also handle transactions committing out of sequence.
    let mut seen = std::collections::HashSet::new();
    if socket
        .send(Message::Text(
            serde_json::json!({"type":"ready"}).to_string().into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let mut interval = tokio::time::interval(Duration::from_secs(3));
    loop {
        tokio::select! {
            incoming=socket.recv()=>match incoming {Some(Ok(Message::Ping(_)|Message::Pong(_)))=>{},_=>break},
            _=interval.tick()=>{
                if !matches!(lisek_db::find_user_by_session_token_hash(&pool,&hash).await,Ok(Some(lisek_core::User { must_change_password:false, .. }))){break;}
                let Ok(events)=lisek_db::notifications::browser_events(&pool,started).await else{break;};
                let present:std::collections::HashSet<_>=events.iter().map(|e|e.event_id).collect();
                seen.retain(|id|present.contains(id));
                let fresh:Vec<_>=events.into_iter().filter(|e|seen.insert(e.event_id)).collect();
                if !fresh.is_empty(){
                    let total=fresh.len();
                    let items:Vec<_>=fresh.into_iter().rev().take(3).collect();
                    let message=serde_json::json!({"type":"library_changed","items":items,"count":total});
                    if socket.send(Message::Text(message.to_string().into())).await.is_err(){break;}
                } else if socket.send(Message::Ping(Vec::new().into())).await.is_err(){break;}
            }
        }
    }
    let _ = socket.send(Message::Close(None)).await;
}
