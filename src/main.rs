mod api;
mod config;
mod crypto;
mod db;
mod embed;
mod imap;
mod mail;
mod storage;

use anyhow::Result;
use config::AppConfig;
use embed::Embedder;
use std::sync::Arc;
use tera::Tera;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let config = AppConfig::from_env()?;
    config.validate_master_key()?;

    println!("Connecting to database...");
    let pool = db::init_pool(&config.database_url).await?;
    println!("Database connected.");

    // Initialize embedder (downloads model on first run)
    println!("Loading embedding model (downloads on first run, ~90MB)...");
    let embeddings = embed::init_embedder().await?;
    let embedder: Embedder = Arc::new(RwLock::new(Some(embeddings)));
    println!("Embedding model loaded.");

    // Create data directories
    std::fs::create_dir_all(format!("{}/attachments", config.data_dir))?;

    // Spawn sync loop as background task
    let sync_pool = pool.clone();
    let sync_config = config.clone();
    let sync_key = config.archive_master_key.clone();
    let sync_embedder = embedder.clone();
    let sync_data_dir = config.data_dir.clone();
    tokio::spawn(async move {
        imap::sync_engine::run_sync_loop(sync_pool, sync_config, sync_key, sync_embedder, sync_data_dir).await;
    });

    // Build and run server
    let tera = match Tera::new("templates/**/*") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Template parsing error(s): {e}");
            std::process::exit(1);
        }
    };

    let app = api::routes::build_router(pool, config.archive_master_key, tera)
        .layer(axum::middleware::from_fn(api::auth_middleware::auth_middleware))
        .layer(axum::Extension(config.jwt_secret.clone()));
    let addr = format!("{}:{}", config.host, config.port);
    println!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
