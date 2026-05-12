# Critical and Important Issues Fix Plan

> **For Hermes:** Use subagent-development skill to implement this plan task-by-task.

**Goal:** Fix critical security vulnerabilities and important functional bugs in the mail-archiver application to make it production-ready.

**Architecture:** 
1. Add authentication middleware to protect all API endpoints
2. Fix database initialization to ensure settings row exists
3. Wire the embedding model properly through to the email processing pipeline
4. Fix SQL injection risk in text search configuration
5. Add proper error handling for unwrap() calls in production code

**Tech Stack:** Rust, Axum, SQLx, Tokio

---

## Task 1: Add Settings Row Initialization

**Objective:** Ensure the settings table always has a row with id=1 to prevent runtime errors when fetching settings.

**Files:**
- Modify: `migrations/20240101000000_create_schema.sql`

**Step 1: Add INSERT statement to migration**

```sql
CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE settings (
    id INT PRIMARY KEY DEFAULT 1,
    max_parallelism INT NOT NULL DEFAULT 4,
    default_sync_interval_seconds INT NOT NULL DEFAULT 300,
    password_hash VARCHAR NOT NULL DEFAULT 'not_yet_configured'
);

-- Enforce single row
CREATE UNIQUE INDEX settings_single_row ON settings((id IS NOT NULL));

-- INSERT initial row if it doesn't exist
INSERT INTO settings (id) VALUES (1) ON CONFLICT DO NOTHING;
```

**Step 2: Verify migration applies correctly**

Run: `docker compose run --rm app sqlx migrate add test_migration` (just to check syntax)
Expected: No syntax errors

**Step 3: Commit**

```bash
git add migrations/20240101000000_create_schema.sql
git commit -m "fix: add initial settings row to migration"
```

---

## Task 2: Implement Authentication Middleware

**Objective:** Add session-based authentication middleware to protect all API endpoints.

**Files:**
- Create: `src/auth/middleware.rs`
- Modify: `src/api/routes.rs`
- Modify: `src/main.rs` (to use middleware)

**Step 1: Create authentication middleware**

```rust
use anyhow::Result;
use axum::{
    async_trait,
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::future::ready;
use tower::Layer;
use tower::Service;

/// Claims structure for our JWT tokens
#[derive(Debug, Deserialize, Serialize)]
struct Claims {
    sub: String,  // subject (username)
    exp: usize,   // expiration timestamp
}

/// Authentication middleware layer
#[derive(Clone)]
pub struct AuthLayer {
    jwt_secret: String,
}

impl AuthLayer {
    pub fn new(jwt_secret: String) -> Self {
        Self { jwt_secret }
    }
}

impl<S> Layer<S> for AuthLayer
where
    S: tower::Service<Request<Body>, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
{
    type Service = AuthMiddleware<S>;
    type Error = S::Error;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware {
            inner,
            jwt_secret: self.jwt_secret.clone(),
        }
    }
}

/// Authentication middleware service
#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    jwt_secret: String,
}

impl<S> tower::Service<Request<Body>> for AuthMiddleware<S>
where
    S: tower::Service<Request<Body>, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = pin_project_lite::pin_project! {
        #[project = Project]
        enum Future {
            S[#S::Future],
            ErrorResponse[std::future::Ready<Result<Response, S::Error>>]
        }
    };

    fn poll_ready(
        &self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&self, mut req: Request<Body>) -> Self::Future {
        // Skip auth for login endpoint and health checks
        let path = req.uri().path();
        if path == "/api/auth/login" || path.starts_with("/health") || path.starts_with("/static/") {
            return Project::S(self.inner.call(req));
        }

        // Extract token from Authorization header
        let auth_header = match req.headers().get(axum::http::header::AUTHORIZATION) {
            Some(header) => header.to_str().unwrap_or(""),
            None => return Project::ErrorResponse(ready(Ok(
                (
                    StatusCode::UNAUTHORIZED,
                    "Missing Authorization header".to_string(),
                )
                    .into_response(),
            ))),
        };

        // Bearer token format
        let token = if auth_header.starts_with("Bearer ") {
            &auth_header[7..]
        } else {
            return Project::ErrorResponse(ready(Ok(
                (
                    StatusCode::UNAUTHORIZED,
                    "Invalid Authorization header format".to_string(),
                )
                    .into_response(),
            )));
        };

        // Validate token
        match decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_ref()),
            &Validation::new(Algorithm::HS256),
        ) {
            Ok(_) => Project::S(self.inner.call(req)),
            Err(_) => Project::ErrorResponse(ready(Ok(
                (
                    StatusCode::UNAUTHORIZED,
                    "Invalid or expired token".to_string(),
                )
                    .into_response(),
            ))),
        }
    }
}
```

**Step 2: Update auth handler to generate proper JWT tokens**

Modify `src/api/auth.rs`:
- Replace `generate_session_token()` with proper JWT generation
- Add JWT generation function

```rust
use jsonwebtoken::{encode, EncodingKey, Header};

// ...

/// Generate JWT token for authenticated user
pub fn generate_jwt_token(username: &str, jwt_secret: &str) -> Result<String> {
    let expiration = chrono::Utc::now()
        .checked_add_signed(chrono::Duration::hours(24))
        .expect("valid timestamp")
        .timestamp() as usize;

    let claims = Claims {
        sub: username.to_string(),
        exp: expiration,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_ref()),
    )
    .map_err(|e| anyhow::anyhow!("Failed to encode JWT: {e}"))
}

// Update login handler to use JWT
// In login_handler, replace:
//   let token = generate_session_token();
// with:
//   let token = generate_jwt_token(&req.password, &jwt_secret)?;
// And change response accordingly
```

**Step 3: Update main.rs to use the middleware**

```rust
// Add imports
use auth::middleware::{AuthLayer, AuthMiddleware};

// After creating the app, before .with_state()
let app = api::routes::build_router(pool, config.archive_master_key, tera)
    .layer(AuthLayer::new(config.jwt_secret.clone()))
    .with_state((pool, config.archive_master_key, tera));
```

**Step 4: Update Cargo.toml to add jsonwebtoken dependency**

```toml
# Auth
jsonwebtoken = "8"
```

**Step 5: Commit**

```bash
git add src/auth/middleware.rs src/api/auth.rs src/main.rs Cargo.toml
git commit -m "feat: add JWT-based authentication middleware"
```

---

## Task 3: Fix Embedder Wiring to Sync Engine

**Objective:** Pass the embedding model through to the email processing pipeline so that emails actually get embeddings and FTS vectors generated.

**Files:**
- Modify: `src/main.rs`
- Modify: `src/imap/sync_engine.rs`
- Modify: `src/db/mod.rs` (if needed for context passing)

**Step 1: Update main.rs to clone and pass embedder to sync loop**

```rust
// After initializing embedder
let embedder_clone = embedder.clone();  // Clone the Arc

// Spawn sync loop as background task - NOW PASS EMBEDDER
let sync_pool = pool.clone();
let sync_config = config.clone();
let sync_key = config.archive_master_key.clone();
let sync_embedder = embedder_clone;    // <-- THIS WAS MISSING
tokio::spawn(async move {
    imap::sync_engine::run_sync_loop(sync_pool, sync_config, sync_key, sync_embedder).await;
});
```

**Step 2: Update sync_engine.rs function signature and usage**

Modify `run_sync_loop` signature:
```rust
pub async fn run_sync_loop(
    pool: PgPool,
    config: AppConfig,
    master_key: String,
    embedder: embed::Embedder,  // ADD THIS PARAMETER
) {
```

Then pass it to `sync_account`:
```rust
tokio::spawn(async move {
    if let Err(e) = sync_account(&account, &pool, &master_key, &semaphore, &embedder).await {
        eprintln!("Sync error for {}: {e}", account.email_address);
    }
});
```

And update `sync_account` function signature:
```rust
pub async fn sync_account(
    account: &MailAccount,
    pool: &PgPool,
    master_key: &str,
    semaphore: &Semaphore,
    embedder: &embed::Embedder,  // ADD THIS
) -> Result<()> {
```

Finally, pass it to `process_and_store_email`:
```rust
if let Err(e) = storage::process_and_store_email(
    pool,
    account.id,
    uid,
    folder,
    &raw_email,
    embedder,  // PASS IT HERE
    &data_dir,
).await
```

**Step 3: Update storage/db.mod.rs to accept embedder parameter**

Actually, looking at the code, `process_and_store_email` in `storage/mod.rs` already accepts an embedder parameter - it's just not being passed from the sync engine. So we just need to update the call sites.

**Step 4: Commit**

```bash
git add src/main.rs src/imap/sync_engine.rs
git commit -m "fix: wire embedder through to email processing pipeline"
```

---

## Task 4: Fix SQL Injection Risk in Text Search Configuration

**Objective:** Replace string interpolation with parameterized queries for the text search configuration to eliminate SQL injection risk.

**Files:**
- Modify: `src/storage/mod.rs`

**Step 1: Replace format!() with proper parameter binding**

In the `process_and_store_email` function, replace:
```rust
sqlx::query(
    &format!(
        "UPDATE emails SET search_vector = $1::vector, fts_doc = to_tsvector('{}', $2) WHERE id = $3",
        ts_config
    ),
)
.bind(&vector_str)
.bind(&fts_input)
.bind(email_id)
```

With:
```rust
sqlx::query(
    "UPDATE emails SET search_vector = $1::vector, fts_doc = to_tsvector($2, $3) WHERE id = $4",
)
.bind(&vector_str)
.bind(ts_config)  // Now a parameter
.bind(&fts_input)
.bind(email_id)
```

And similarly for the else branch:
```rust
sqlx::query(&format!(
    "UPDATE emails SET fts_doc = to_tsvector('{}', $1) WHERE id = $2",
    ts_config
))
```

Becomes:
```rust
sqlx::query("UPDATE emails SET fts_doc = to_tsvector($1, $2) WHERE id = $3")
    .bind(ts_config)
    .bind(&fts_input)
    .bind(email_id)
```

**Step 2: Commit**

```bash
git add src/storage/mod.rs
git commit -m "fix: eliminate SQL injection risk in text search configuration"
```

---

## Task 5: Replace unwrap() Calls with Proper Error Handling

**Objective:** Replace panic-inducing unwrap() calls in production code with proper error handling.

**Files:**
- Modify: `src/mail/language.rs`
- Modify: `src/mail/parser.rs`

**Step 1: Fix language.rs unwrap() calls**

Replace both instances of:
```rust
let lang = detect_language(text).unwrap();
```

With:
```rust
let lang = detect_language(text).unwrap_or_else(|| "unknown".to_string());
```

**Step 2: Fix parser.rs unwrap() calls**

Replace:
```rust
let selector = Selector::parse("script, style").unwrap();
```

With:
```rust
let selector = Selector::parse("script, style").unwrap_or_else(|_| {
    // Fallback to a simple selector that matches nothing
    Selector::parse("").unwrap()
});
```

And replace:
```rust
let email = ParsedEmail::from_bytes(raw).unwrap();
```

With proper error handling - if we can't parse the email, we should skip it or log an error:
```rust
let email = match ParsedEmail::from_bytes(raw) {
    Ok(e) => e,
    Err(e) {
        eprintln!("Failed to parse email: {e}");
        continue; // or return an error depending on context
    }
};
```

**Step 3: Commit**

```bash
git add src/mail/language.rs src/mail/parser.rs
git commit -m "fix: replace unwrap() calls with proper error handling"
```

---

## Verification Steps

After completing all tasks, run these verification steps:

1. **Compile check:** `cargo check` should pass with no warnings
2. **Unit tests:** `cargo test` should pass
3. **Integration test:** Start the application with docker compose and verify:
   - Login endpoint works and returns JWT token
   - Protected endpoints require Authorization header
   - Settings can be fetched and updated
   - Email processing includes embeddings (check database for non-null search_vector)
   - No panics on malformed input

**Final commit:**
```bash
git add .
git commit -m "fix: resolve all critical and important issues"
```

---

**Note:** These fixes address the most critical security and functionality issues. Minor issues like additional unwrap() calls, test coverage, and dependency updates should be addressed in follow-up work.