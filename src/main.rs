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
    let _embedder: Embedder = Arc::new(RwLock::new(Some(embeddings)));
    println!("Embedding model loaded.");

    // Create data directories
    std::fs::create_dir_all(format!("{}/attachments", config.data_dir))?;

    // Build and run the server
    let app = api::routes::build_router(pool);
    let addr = format!("{}:{}", config.host, config.port);
    println!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
