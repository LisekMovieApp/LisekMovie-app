use crate::{PgPool,DbError};
use serde::{Serialize,Deserialize};
use uuid::Uuid;

#[derive(Serialize,sqlx::FromRow,utoipa::ToSchema)]
pub struct Endpoint {
    pub id:Uuid,pub name:String,pub url:String,pub destination_type:String,
    pub subscribed_events:Vec<String>,pub enabled:bool,
}
#[derive(Deserialize,utoipa::ToSchema)]
pub struct EndpointInput {
    pub name:String,pub url:String,pub destination_type:String,
    pub subscribed_events:Vec<String>,pub enabled:bool,
}
pub async fn endpoints(pool:&PgPool)->Result<Vec<Endpoint>,DbError>{
    Ok(sqlx::query_as("SELECT id,name,url,destination_type,subscribed_events,enabled FROM webhook_endpoints ORDER BY created_at")
        .fetch_all(pool).await?)
}
pub async fn create(pool:&PgPool,e:&EndpointInput)->Result<Uuid,DbError>{
    let (id,):(Uuid,)=sqlx::query_as("INSERT INTO webhook_endpoints(name,url,destination_type,subscribed_events,enabled) VALUES ($1,$2,$3,$4,$5) RETURNING id")
        .bind(&e.name).bind(&e.url).bind(&e.destination_type).bind(&e.subscribed_events).bind(e.enabled).fetch_one(pool).await?;
    Ok(id)
}
pub async fn delete(pool:&PgPool,id:Uuid)->Result<bool,DbError>{
    Ok(sqlx::query("DELETE FROM webhook_endpoints WHERE id=$1").bind(id).execute(pool).await?.rows_affected()>0)
}
#[derive(sqlx::FromRow)]
pub struct Delivery {pub id:Uuid,pub url:String,pub destination_type:String,pub payload:serde_json::Value,pub attempt_count:i32}
pub async fn claim(pool:&PgPool)->Result<Option<Delivery>,DbError>{
    Ok(sqlx::query_as("WITH candidate AS (SELECT d.id FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id WHERE e.enabled AND d.status IN ('pending','processing') AND d.next_attempt_at<=now() ORDER BY d.next_attempt_at LIMIT 1 FOR UPDATE OF d SKIP LOCKED), claimed AS (UPDATE webhook_deliveries d SET status='processing',next_attempt_at=now()+interval '60 seconds',attempt_count=attempt_count+1,last_attempt_at=now() FROM candidate c WHERE d.id=c.id RETURNING d.*) SELECT c.id,e.url,e.destination_type,c.payload,c.attempt_count FROM claimed c JOIN webhook_endpoints e ON e.id=c.endpoint_id").fetch_optional(pool).await?)
}
pub async fn complete(pool:&PgPool,id:Uuid,status:Option<u16>,error:Option<&str>,attempt:i32,delay:i64)->Result<(),DbError>{
    let mut tx=pool.begin().await?;
    sqlx::query("INSERT INTO webhook_delivery_attempts(delivery_id,response_code,error_code) VALUES ($1,$2,$3)").bind(id).bind(status.map(i32::from)).bind(error).execute(&mut *tx).await?;
    let delivered=status.is_some_and(|s|(200..300).contains(&s));
    sqlx::query("UPDATE webhook_deliveries SET status=$2,response_code=$3,next_attempt_at=now()+$4::bigint*interval '1 second' WHERE id=$1")
        .bind(id).bind(if delivered{"delivered"}else if attempt>=5{"failed"}else{"pending"}).bind(status.map(i32::from)).bind(delay).execute(&mut *tx).await?;
    tx.commit().await?;Ok(())
}
pub async fn deliveries(pool:&PgPool)->Result<serde_json::Value,DbError>{
    let (rows,):(serde_json::Value,)=sqlx::query_as("SELECT COALESCE(jsonb_agg(r),'[]'::jsonb) FROM (SELECT d.id,e.name,d.event_type,d.status,d.attempt_count,d.response_code,d.created_at,(SELECT COALESCE(jsonb_agg(a ORDER BY a.attempted_at),'[]'::jsonb) FROM webhook_delivery_attempts a WHERE a.delivery_id=d.id) AS attempts FROM webhook_deliveries d JOIN webhook_endpoints e ON e.id=d.endpoint_id ORDER BY d.created_at DESC LIMIT 100) r").fetch_one(pool).await?;
    Ok(rows)
}


#[derive(serde::Serialize,sqlx::FromRow)]
pub struct BrowserEvent { pub event_id:i64, pub item_id:uuid::Uuid, pub title:String }
/// New-item events since `since` that the account may see.
pub async fn browser_events(pool:&PgPool,since:chrono::DateTime<chrono::Utc>,access:&crate::access::Restrictions)->Result<Vec<BrowserEvent>,DbError>{
    let (libraries,max_age,block_unrated)=access.binds();
    Ok(sqlx::query_as("SELECT e.id AS event_id,e.item_id,m.title FROM browser_notification_events e JOIN media_items m ON m.id=e.item_id WHERE e.created_at >= $1 AND e.created_at > now()-interval '5 minutes' AND lisek_visible(m.library_id,m.age_rating,$2,$3,$4) ORDER BY e.id DESC LIMIT 1000")
        .bind(since).bind(libraries).bind(max_age).bind(block_unrated).fetch_all(pool).await?.into_iter().rev().collect())
}
pub async fn prune_browser_events(pool:&PgPool)->Result<(),DbError>{
    sqlx::query("DELETE FROM browser_notification_events WHERE created_at < now()-interval '1 day'").execute(pool).await?;
    Ok(())
}
