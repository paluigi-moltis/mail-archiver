use axum::{
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use super::auth::login_handler;

pub fn build_router(pool: PgPool) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/api/auth/login", post(login_handler))
        .with_state(pool)
}

async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "mail-archive"
    }))
}
