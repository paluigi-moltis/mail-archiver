use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

/// Paths that don't require authentication
const PUBLIC_PATHS: &[&str] = &["/api/auth/login", "/health", "/", "/stats"];

pub async fn auth_middleware(
    Extension(jwt_secret): Extension<String>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Allow public paths and static assets
    if PUBLIC_PATHS.contains(&path) || path.starts_with("/static/") {
        return next.run(request).await;
    }

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Validate Bearer token
    let token = if let Some(t) = auth_header.strip_prefix("Bearer ") {
        t
    } else {
        return (
            StatusCode::UNAUTHORIZED,
            "Missing or invalid Authorization header",
        )
            .into_response();
    };

    match decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    ) {
        Ok(_) => next.run(request).await,
        Err(_) => (StatusCode::UNAUTHORIZED, "Invalid or expired token").into_response(),
    }
}
