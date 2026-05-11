mod api;
mod config;
mod db;
mod embed;
mod imap;
mod mail;
mod storage;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let database_url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL environment variable must be set");

    println!("Connecting to database...");
    let _pool = db::init_pool(&database_url).await?;
    println!("DB connected successfully");

    Ok(())
}
