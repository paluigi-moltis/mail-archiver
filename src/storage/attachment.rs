#![allow(dead_code)]

use anyhow::Result;
use sqlx::PgPool;
use std::path::PathBuf;

use crate::mail::parser::AttachmentInfo;

/// Save an attachment to disk using content-addressable storage.
/// Returns the SHA-256 hash if the file was saved (or already existed).
pub async fn save_attachment(
    pool: &PgPool,
    email_id: i32,
    attachment: &AttachmentInfo,
    data_dir: &str,
) -> Result<String> {
    let hash = &attachment.file_hash;
    let prefix = &hash[..2];

    // Build path: data_dir/attachments/ab/abcdef...
    let dir = PathBuf::from(data_dir).join("attachments").join(prefix);
    let file_path = dir.join(hash);

    // Write file only if it doesn't exist
    if !file_path.exists() {
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(&file_path, &attachment.data).await?;
    }

    // Insert into DB
    sqlx::query(
        "INSERT INTO attachments (email_id, filename, mime_type, size_bytes, file_hash, is_inline) 
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(email_id)
    .bind(&attachment.filename)
    .bind(&attachment.mime_type)
    .bind(attachment.data.len() as i64)
    .bind(hash)
    .bind(attachment.is_inline)
    .execute(pool)
    .await?;

    Ok(hash.clone())
}

/// Get the file path for an attachment by its hash.
pub fn attachment_path(data_dir: &str, hash: &str) -> PathBuf {
    let prefix = &hash[..2];
    PathBuf::from(data_dir)
        .join("attachments")
        .join(prefix)
        .join(hash)
}

/// Clean up orphaned attachments after email deletion.
/// Called after DELETE FROM emails — checks if any attachment hashes
/// are no longer referenced by any email.
pub async fn cleanup_orphaned_attachments(pool: &PgPool, data_dir: &str) -> Result<u64> {
    // Find attachment hashes that have no referencing email
    let orphaned: Vec<String> = sqlx::query_scalar(
        "SELECT a.file_hash FROM attachments a 
         WHERE NOT EXISTS (SELECT 1 FROM emails e WHERE e.id = a.email_id)",
    )
    .fetch_all(pool)
    .await?;

    let mut cleaned = 0u64;
    for hash in &orphaned {
        let path = attachment_path(data_dir, hash);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
            cleaned += 1;
        }
    }

    if !orphaned.is_empty() {
        // Delete orphaned rows from attachments table
        sqlx::query("DELETE FROM attachments WHERE file_hash = ANY($1)")
            .bind(&orphaned)
            .execute(pool)
            .await?;
    }

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attachment_path() {
        let hash = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let path = attachment_path("/data", hash);
        assert_eq!(
            path.to_string_lossy(),
            "/data/attachments/ab/abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
        );
    }
}
