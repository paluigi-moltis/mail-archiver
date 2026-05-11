use std::sync::Arc;
use tokio::sync::RwLock;

use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub type Embedder = Arc<RwLock<Option<TextEmbedding>>>;

pub async fn init_embedder() -> Result<TextEmbedding> {
    // This downloads the model on first run (~90MB)
    let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2Q))?;
    Ok(model)
}

#[allow(dead_code)]
pub async fn embed(text: &str, embedder: &Embedder) -> Result<Vec<f32>> {
    let guard = embedder.read().await;
    let model = guard
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Embedder not initialized"))?;
    let result = model.embed(vec![text.to_string()], None)?;
    Ok(result[0].clone())
}
