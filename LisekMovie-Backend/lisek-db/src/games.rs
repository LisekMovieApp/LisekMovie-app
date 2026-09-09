use crate::{PgPool,DbError};
use serde::Serialize;
use uuid::Uuid;
#[derive(Serialize,sqlx::FromRow,utoipa::ToSchema)]
pub struct Game { pub game_key:String, pub name:String, pub enabled:bool }
#[derive(Serialize,sqlx::FromRow,utoipa::ToSchema)]
pub struct Score { pub username:String, pub score:i32 }
pub async fn list(pool:&PgPool,admin:bool)->Result<Vec<Game>,DbError>{
    Ok(sqlx::query_as("SELECT game_key,name,enabled FROM games WHERE enabled OR $1 ORDER BY name").bind(admin).fetch_all(pool).await?)
}
pub async fn set_enabled(pool:&PgPool,key:&str,enabled:bool)->Result<bool,DbError>{
    Ok(sqlx::query("UPDATE games SET enabled=$2 WHERE game_key=$1").bind(key).bind(enabled).execute(pool).await?.rows_affected()>0)
}
pub async fn enabled(pool:&PgPool,key:&str)->Result<bool,DbError>{
    let (enabled,):(bool,)=sqlx::query_as("SELECT EXISTS(SELECT 1 FROM games WHERE game_key=$1 AND enabled)").bind(key).fetch_one(pool).await?;
    Ok(enabled)
}
pub async fn save_score(pool:&PgPool,key:&str,user:Uuid,score:i32)->Result<bool,DbError>{
    Ok(sqlx::query("INSERT INTO game_scores(game_key,user_id,score) SELECT game_key,$2,$3 FROM games WHERE game_key=$1 AND enabled").bind(key).bind(user).bind(score).execute(pool).await?.rows_affected()>0)
}
pub async fn leaderboard(pool:&PgPool,key:&str)->Result<Vec<Score>,DbError>{
    Ok(sqlx::query_as("SELECT u.username,MAX(s.score) AS score FROM game_scores s JOIN users u ON u.id=s.user_id WHERE game_key=$1 GROUP BY u.id,u.username ORDER BY score DESC,u.username LIMIT 50").bind(key).fetch_all(pool).await?)
}
