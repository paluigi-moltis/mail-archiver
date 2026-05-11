use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool};

use super::auth::login_handler;
use crate::crypto;

// ─── Request / Response types ───────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub email_address: String,
    pub label: Option<String>,
    pub imap_server: String,
    pub imap_port: Option<i32>,
    pub use_tls: Option<bool>,
    pub password: String,
    pub excluded_folders: Option<Vec<String>>,
    pub grace_period_days: Option<i32>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AccountResponse {
    pub id: i32,
    pub email_address: String,
    pub label: Option<String>,
    pub imap_server: String,
    pub imap_port: i32,
    pub use_tls: bool,
    pub excluded_folders: Value,
    pub grace_period_days: i32,
    pub last_sync_at: Option<chrono::DateTime<chrono::Utc>>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub max_parallelism: Option<i32>,
    pub default_sync_interval_seconds: Option<i32>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct SettingsResponse {
    pub id: i32,
    pub max_parallelism: i32,
    pub default_sync_interval_seconds: i32,
}

/// The shared application state passed to all handlers.
pub type AppState = (PgPool, String); // (pool, master_key)

// ─── Router ─────────────────────────────────────────────────────────

pub fn build_router(pool: PgPool, master_key: String) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/api/auth/login", post(login_handler))
        .route(
            "/api/accounts",
            post(create_account_handler).get(list_accounts_handler),
        )
        .route("/api/accounts/{id}", delete(delete_account_handler))
        .route(
            "/api/settings",
            get(get_settings_handler).put(update_settings_handler),
        )
        .with_state((pool, master_key))
}

// ─── Health ─────────────────────────────────────────────────────────

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "mail-archive"
    }))
}

// ─── Account CRUD ───────────────────────────────────────────────────

/// POST /api/accounts — create a new IMAP account.
pub async fn create_account_handler(
    State((pool, master_key)): State<AppState>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let (encrypted_password, encryption_nonce) = crypto::encrypt(&master_key, &req.password)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let excluded = serde_json::to_value(
        req.excluded_folders
            .unwrap_or_else(|| vec!["Spam".into(), "Trash".into()]),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let row: AccountResponse = sqlx::query_as(
        r#"INSERT INTO mail_accounts (email_address, label, imap_server, imap_port, use_tls,
           encrypted_password, encryption_nonce, excluded_folders, grace_period_days)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id, email_address, label, imap_server, imap_port, use_tls,
           excluded_folders, grace_period_days, last_sync_at, enabled"#,
    )
    .bind(&req.email_address)
    .bind(&req.label)
    .bind(&req.imap_server)
    .bind(req.imap_port.unwrap_or(993))
    .bind(req.use_tls.unwrap_or(true))
    .bind(&encrypted_password)
    .bind(&encryption_nonce)
    .bind(&excluded)
    .bind(req.grace_period_days.unwrap_or(30))
    .fetch_one(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(row))
}

/// GET /api/accounts — list all accounts.
pub async fn list_accounts_handler(
    State((pool, _master_key)): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let rows: Vec<AccountResponse> = sqlx::query_as(
        r#"SELECT id, email_address, label, imap_server, imap_port, use_tls,
           excluded_folders, grace_period_days, last_sync_at, enabled
           FROM mail_accounts ORDER BY id"#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rows))
}

/// DELETE /api/accounts/:id — delete an account and all its emails.
pub async fn delete_account_handler(
    State((pool, _master_key)): State<AppState>,
    Path(id): Path<i32>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let result = sqlx::query("DELETE FROM mail_accounts WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, format!("Account {id} not found")));
    }

    Ok(Json(json!({"deleted": id})))
}

// ─── Settings ───────────────────────────────────────────────────────

/// GET /api/settings — get current application settings.
pub async fn get_settings_handler(
    State((pool, _master_key)): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let row: SettingsResponse = sqlx::query_as(
        r#"SELECT id, max_parallelism, default_sync_interval_seconds
           FROM settings WHERE id = 1"#,
    )
    .fetch_one(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(row))
}

/// PUT /api/settings — update application settings.
pub async fn update_settings_handler(
    State((pool, _master_key)): State<AppState>,
    Json(req): Json<UpdateSettingsRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if let Some(max) = req.max_parallelism {
        sqlx::query("UPDATE settings SET max_parallelism = $1 WHERE id = 1")
            .bind(max)
            .execute(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    if let Some(interval) = req.default_sync_interval_seconds {
        sqlx::query("UPDATE settings SET default_sync_interval_seconds = $1 WHERE id = 1")
            .bind(interval)
            .execute(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(json!({"status": "updated"})))
}
