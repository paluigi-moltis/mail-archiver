use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct AppConfig {
    pub database_url: String,
    pub archive_master_key: String, // 32-byte hex string
    pub max_workers: usize,
    pub host: String,
    pub port: u16,
    pub data_dir: String, // base directory for attachments
    pub jwt_secret: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: std::env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL must be set"))?,
            archive_master_key: std::env::var("ARCHIVE_MASTER_KEY").map_err(|_| {
                anyhow::anyhow!("ARCHIVE_MASTER_KEY must be set (32-byte hex string)")
            })?,
            max_workers: std::env::var("MAX_WORKERS")
                .unwrap_or_else(|_| "8".to_string())
                .parse()?,
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8000".to_string())
                .parse()?,
            data_dir: std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".to_string()),
            jwt_secret: std::env::var("JWT_SECRET")
                .map_err(|_| anyhow::anyhow!("JWT_SECRET must be set"))?,
        })
    }

    /// Validate the master key is exactly 64 hex chars (32 bytes)
    pub fn validate_master_key(&self) -> Result<()> {
        if self.archive_master_key.len() != 64 {
            anyhow::bail!("ARCHIVE_MASTER_KEY must be a 32-byte hex string (64 characters)");
        }
        hex::decode(&self.archive_master_key)
            .map_err(|_| anyhow::anyhow!("ARCHIVE_MASTER_KEY must be valid hex"))?;
        Ok(())
    }
}
