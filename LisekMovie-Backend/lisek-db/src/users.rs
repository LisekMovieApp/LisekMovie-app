//! Account writes serialize role decisions and recheck the acting session inside the transaction.
use crate::{DbError, PgPool, UserRow, user_from_row};
use lisek_core::User;
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("database operation failed")]
    Database(#[from] DbError),
    #[error("admin_required")]
    Forbidden,
    #[error("user_not_found")]
    NotFound,
    #[error("last_administrator")]
    LastAdmin,
    #[error("credentials_changed")]
    Changed,
}
impl From<sqlx::Error> for AccountError { fn from(e:sqlx::Error)->Self{Self::Database(e.into())} }

pub async fn list(pool:&PgPool)->Result<Vec<User>,DbError>{
    Ok(sqlx::query_as::<_,UserRow>("SELECT id,username,is_admin,ui_language,email,created_at,must_change_password,disabled FROM users ORDER BY lower(username),id")
        .fetch_all(pool).await?.into_iter().map(user_from_row).collect())
}
async fn admin_tx(pool:&PgPool,actor_hash:&str)->Result<sqlx::Transaction<'static,sqlx::Postgres>,AccountError>{
    let mut tx=pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(710020)").execute(&mut *tx).await?;
    let (allowed,):(bool,)=sqlx::query_as("SELECT EXISTS(SELECT 1 FROM users u JOIN sessions s ON s.user_id=u.id WHERE s.token_hash=$1 AND s.expires_at>now() AND u.is_admin AND NOT u.disabled AND NOT u.must_change_password)")
        .bind(actor_hash).fetch_one(&mut *tx).await?;
    if !allowed{return Err(AccountError::Forbidden);}
    Ok(tx)
}
pub async fn create(pool:&PgPool,actor:&str,name:&str,hash:&str,admin:bool,language:&str)->Result<User,AccountError>{
    let mut tx=admin_tx(pool,actor).await?;
    let row=sqlx::query_as::<_,UserRow>("INSERT INTO users(username,password_hash,is_admin,ui_language,must_change_password) VALUES ($1,$2,$3,$4,true) RETURNING id,username,is_admin,ui_language,email,created_at,must_change_password,disabled")
        .bind(name).bind(hash).bind(admin).bind(language).fetch_one(&mut *tx).await?;
    tx.commit().await?;
    Ok(user_from_row(row))
}
pub async fn edit(pool:&PgPool,actor:&str,id:Uuid,name:&str,admin:bool,disabled:bool,language:&str)->Result<User,AccountError>{
    let mut tx=admin_tx(pool,actor).await?;
    let old:Option<(bool,bool)>=sqlx::query_as("SELECT is_admin,disabled FROM users WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let Some((was_admin,was_disabled))=old else{return Err(AccountError::NotFound);};
    if was_admin && !was_disabled && (!admin || disabled) {
        let (others,):(i64,)=sqlx::query_as("SELECT count(*) FROM users WHERE is_admin AND NOT disabled AND id<>$1").bind(id).fetch_one(&mut *tx).await?;
        if others==0{return Err(AccountError::LastAdmin);}
    }
    let row=sqlx::query_as::<_,UserRow>("UPDATE users SET username=$2,is_admin=$3,disabled=$4,ui_language=$5,updated_at=now() WHERE id=$1 RETURNING id,username,is_admin,ui_language,email,created_at,must_change_password,disabled")
        .bind(id).bind(name).bind(admin).bind(disabled).bind(language).fetch_one(&mut *tx).await?;
    if disabled || admin!=was_admin {
        sqlx::query("DELETE FROM sessions WHERE user_id=$1").bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(user_from_row(row))
}
pub async fn reset_password(pool:&PgPool,actor:&str,id:Uuid,hash:&str)->Result<(),AccountError>{
    let mut tx=admin_tx(pool,actor).await?;
    let result=sqlx::query("UPDATE users SET password_hash=$2,must_change_password=true,updated_at=now() WHERE id=$1").bind(id).bind(hash).execute(&mut *tx).await?;
    if result.rows_affected()==0{return Err(AccountError::NotFound);}
    sqlx::query("DELETE FROM sessions WHERE user_id=$1").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}
/// Prevent a login verified before a password reset from creating a session after that reset.
pub async fn login_session(pool:&PgPool,id:Uuid,expected_hash:&str,token_hash:&str,client:&str,device:&str,device_id:&str)->Result<bool,DbError>{
    let mut tx=pool.begin().await?;
    let row:Option<(String,bool,bool)>=sqlx::query_as("SELECT password_hash,disabled,must_change_password FROM users WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let Some((hash,false,change))=row else{return Ok(false);};
    if hash!=expected_hash{return Ok(false);}
    sqlx::query("INSERT INTO sessions(user_id,token_hash,client,device,device_id,expires_at) VALUES ($1,$2,$3,$4,$5,now()+$6::bigint*interval '1 second')")
        .bind(id).bind(token_hash).bind(client).bind(device).bind(device_id).bind(if change{900i64}else{2592000i64}).execute(&mut *tx).await?;
    tx.commit().await?; Ok(true)
}
pub async fn credential_hash(pool:&PgPool,id:Uuid)->Result<Option<String>,DbError>{
    Ok(sqlx::query_scalar("SELECT password_hash FROM users WHERE id=$1 AND NOT disabled").bind(id).fetch_optional(pool).await?)
}
/// Atomically consume the current credential/session and issue one replacement session.
pub async fn change_password(pool:&PgPool,id:Uuid,session_hash:&str,expected_hash:&str,new_hash:&str,new_token_hash:&str)->Result<User,AccountError>{
    let mut tx=pool.begin().await?;
    let row:Option<(String,bool)>=sqlx::query_as("SELECT password_hash,disabled FROM users WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    if !matches!(row,Some((ref hash,false)) if hash==expected_hash){return Err(AccountError::Changed);}
    let session:Option<(String,String,String)>=sqlx::query_as("SELECT client,device,device_id FROM sessions WHERE user_id=$1 AND token_hash=$2 AND expires_at>now()")
        .bind(id).bind(session_hash).fetch_optional(&mut *tx).await?;
    let Some((client,device,device_id))=session else{return Err(AccountError::Changed);};
    let row=sqlx::query_as::<_,UserRow>("UPDATE users SET password_hash=$2,must_change_password=false,updated_at=now() WHERE id=$1 RETURNING id,username,is_admin,ui_language,email,created_at,must_change_password,disabled")
        .bind(id).bind(new_hash).fetch_one(&mut *tx).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id=$1").bind(id).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO sessions(user_id,token_hash,client,device,device_id,expires_at) VALUES ($1,$2,$3,$4,$5,now()+interval '30 days')")
        .bind(id).bind(new_token_hash).bind(client).bind(device).bind(device_id).execute(&mut *tx).await?;
    tx.commit().await?; Ok(user_from_row(row))
}

/// Whether any account exists. A database handed to the server through
/// `DATABASE_URL` (Docker) starts empty: the wizard then only creates the
/// first administrator.
pub async fn any_user_exists(pool: &PgPool) -> Result<bool, DbError> {
    let (exists,): (bool,) = sqlx::query_as("SELECT EXISTS (SELECT 1 FROM users)").fetch_one(pool).await?;
    Ok(exists)
}
