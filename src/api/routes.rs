use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{FromRow, PgPool};

use super::auth::login_handler;
use crate::crypto;
use crate::storage::attachment::attachment_path;

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

// ─── Search types ───────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub tags: Option<String>,  // comma-separated tag names
    pub page: Option<u32>,     // default 1
    pub per_page: Option<u32>, // default 20
}

#[derive(Debug, Serialize)]
pub struct SearchResult {
    pub id: i32,
    pub subject: Option<String>,
    pub sender: Option<String>,
    pub received_at: Option<chrono::DateTime<chrono::Utc>>,
    pub body_snippet: Option<String>,
    pub score: f64,
    pub tags: Vec<String>,
    pub has_attachment: bool,
    pub folder: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub total: i64,
    pub page: u32,
    pub per_page: u32,
    pub total_pages: u32,
}

// ─── Stats types ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total_emails: i64,
    pub total_attachments: i64,
    pub attachment_size_bytes: i64,
    pub db_size_bytes: i64,
}

// ─── Tags types ─────────────────────────────────────────────────────

#[derive(Debug, Serialize, FromRow)]
pub struct TagItem {
    pub id: i32,
    pub name: String,
    pub is_auto: bool,
    pub email_count: i64,
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
        .route("/api/search", get(search_handler))
        .route("/api/attachments/{hash}", get(attachment_download_handler))
        .route("/api/stats", get(stats_handler))
        .route("/api/tags", get(tags_handler))
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

// ─── Search ─────────────────────────────────────────────────────────

/// GET /api/search — full-text search with tag filtering and pagination.
pub async fn search_handler(
    State((pool, _master_key)): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let page = params.page.unwrap_or(1).max(1);
    let per_page = params.per_page.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * per_page;

    // Parse optional tag filter into a Vec
    let tag_filter: Vec<String> = params
        .tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Build the count query
    let (count_sql, _count_params): (&str, Vec<String>) = if tag_filter.is_empty() {
        (
            "SELECT COUNT(*) FROM emails e WHERE e.fts_doc @@ plainto_tsquery($1)",
            vec![params.q.clone()],
        )
    } else {
        (
            "SELECT COUNT(*) FROM emails e
             WHERE e.fts_doc @@ plainto_tsquery($1)
               AND NOT EXISTS (
                   SELECT 1 FROM unnest($2::text[]) AS excluded_tag(name)
                   WHERE NOT EXISTS (
                       SELECT 1 FROM email_tags et JOIN tags t ON et.tag_id = t.id
                       WHERE et.email_id = e.id AND t.name = excluded_tag.name
                   )
               )",
            vec![params.q.clone(), tag_filter.join(",")],
        )
    };

    // Count total matching results
    let total: i64 = if tag_filter.is_empty() {
        sqlx::query_scalar(count_sql)
            .bind(&params.q)
            .fetch_one(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        sqlx::query_scalar(count_sql)
            .bind(&params.q)
            .bind(&tag_filter)
            .fetch_one(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    let total_pages = if total == 0 {
        0
    } else {
        (total as u32).div_ceil(per_page)
    };

    // Build the search query
    let search_sql = if tag_filter.is_empty() {
        r#"SELECT e.id, e.subject, e.sender, e.received_at, e.body_text, e.folder,
                  ts_rank_cd(e.fts_doc, plainto_tsquery($1)) AS score
           FROM emails e
           WHERE e.fts_doc @@ plainto_tsquery($1)
           ORDER BY score DESC
           LIMIT $2 OFFSET $3"#
    } else {
        r#"SELECT e.id, e.subject, e.sender, e.received_at, e.body_text, e.folder,
                  ts_rank_cd(e.fts_doc, plainto_tsquery($1)) AS score
           FROM emails e
           WHERE e.fts_doc @@ plainto_tsquery($1)
             AND NOT EXISTS (
                 SELECT 1 FROM unnest($4::text[]) AS excluded_tag(name)
                 WHERE NOT EXISTS (
                     SELECT 1 FROM email_tags et JOIN tags t ON et.tag_id = t.id
                     WHERE et.email_id = e.id AND t.name = excluded_tag.name
                 )
             )
           ORDER BY score DESC
           LIMIT $2 OFFSET $3"#
    };

    #[derive(Debug, FromRow)]
    struct EmailRow {
        id: i32,
        subject: Option<String>,
        sender: Option<String>,
        received_at: Option<chrono::DateTime<chrono::Utc>>,
        body_text: Option<String>,
        folder: String,
        score: f64,
    }

    let rows: Vec<EmailRow> = if tag_filter.is_empty() {
        sqlx::query_as(search_sql)
            .bind(&params.q)
            .bind(per_page as i64)
            .bind(offset as i64)
            .fetch_all(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        sqlx::query_as(search_sql)
            .bind(&params.q)
            .bind(per_page as i64)
            .bind(offset as i64)
            .bind(&tag_filter)
            .fetch_all(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    // For each result, check if it has attachments and get its tags
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        // Get tags for this email
        let tags: Vec<String> = sqlx::query_scalar(
            "SELECT t.name FROM tags t JOIN email_tags et ON t.id = et.tag_id WHERE et.email_id = $1 ORDER BY t.name",
        )
        .bind(row.id)
        .fetch_all(&pool)
        .await
        .unwrap_or_default();

        // Check if email has attachments
        let has_attachment: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE email_id = $1)")
                .bind(row.id)
                .fetch_one(&pool)
                .await
                .unwrap_or(false);

        // Build body snippet (first 200 chars)
        let body_snippet = row.body_text.as_ref().map(|text| {
            let clean: String = text.chars().take(200).collect();
            let trimmed = clean.trim_end();
            if text.len() > 200 {
                format!("{trimmed}…")
            } else {
                trimmed.to_string()
            }
        });

        results.push(SearchResult {
            id: row.id,
            subject: row.subject,
            sender: row.sender,
            received_at: row.received_at,
            body_snippet,
            score: row.score,
            tags,
            has_attachment,
            folder: row.folder,
        });
    }

    Ok(Json(SearchResponse {
        results,
        total,
        page,
        per_page,
        total_pages,
    }))
}

// ─── Attachment download ────────────────────────────────────────────

/// GET /api/attachments/:hash — stream an attachment file from disk.
pub async fn attachment_download_handler(
    State((pool, _master_key)): State<AppState>,
    Path(hash): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let data_dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_string());
    let file_path = attachment_path(&data_dir, &hash);

    if !file_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Attachment not found".to_string()));
    }

    // Look up attachment metadata from DB for filename and mime_type
    #[derive(Debug, FromRow)]
    struct AttachmentMeta {
        filename: String,
        mime_type: Option<String>,
    }

    let meta: Option<AttachmentMeta> =
        sqlx::query_as("SELECT filename, mime_type FROM attachments WHERE file_hash = $1 LIMIT 1")
            .bind(&hash)
            .fetch_optional(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (filename, content_type) = match meta {
        Some(m) => (
            m.filename,
            m.mime_type
                .unwrap_or_else(|| "application/octet-stream".to_string()),
        ),
        None => (hash.clone(), "application/octet-stream".to_string()),
    };

    let file = tokio::fs::File::open(&file_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let response = axum::http::Response::builder()
        .header(header::CONTENT_TYPE, &content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\""),
        )
        .body(body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(response)
}

// ─── Stats ──────────────────────────────────────────────────────────

/// GET /api/stats — return archive statistics.
pub async fn stats_handler(
    State((pool, _master_key)): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let total_emails: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM emails")
        .fetch_one(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total_attachments: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM attachments")
        .fetch_one(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let attachment_size_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM attachments")
            .fetch_one(&pool)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let db_size_bytes: i64 = sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_one(&pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(StatsResponse {
        total_emails,
        total_attachments,
        attachment_size_bytes,
        db_size_bytes,
    }))
}

// ─── Tags ───────────────────────────────────────────────────────────

/// GET /api/tags — list all tags with usage counts.
pub async fn tags_handler(
    State((pool, _master_key)): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let tags: Vec<TagItem> = sqlx::query_as(
        r#"SELECT t.id, t.name, t.is_auto,
                  COUNT(et.email_id) AS email_count
           FROM tags t
           LEFT JOIN email_tags et ON t.id = et.tag_id
           GROUP BY t.id, t.name, t.is_auto
           ORDER BY email_count DESC, t.name"#,
    )
    .fetch_all(&pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(tags))
}
