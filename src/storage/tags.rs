#![allow(dead_code)]

use anyhow::Result;
use sqlx::PgPool;

/// Generate and insert auto-tags for an email.
/// Tags are based on: folder name, sender address, has_attachment flag.
pub async fn auto_tag_email(
    pool: &PgPool,
    email_id: i32,
    folder: &str,
    sender: &str,
    has_attachment: bool,
) -> Result<Vec<String>> {
    let mut tag_names = Vec::new();

    // Folder tag: folder:Inbox
    tag_names.push(format!("folder:{}", folder));

    // Sender tag: from:sender@domain.com
    let sender_email = extract_email_address(sender);
    if !sender_email.is_empty() {
        tag_names.push(format!("from:{}", sender_email));
    }

    // Attachment tag
    if has_attachment {
        tag_names.push("has:attachment".to_string());
    }

    // Insert tags and link them
    for tag_name in &tag_names {
        let tag_id = ensure_tag_exists(pool, tag_name, true).await?;

        sqlx::query(
            "INSERT INTO email_tags (email_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(email_id)
        .bind(tag_id)
        .execute(pool)
        .await?;
    }

    Ok(tag_names)
}

/// Ensure a tag exists in the tags table, return its ID.
async fn ensure_tag_exists(pool: &PgPool, name: &str, is_auto: bool) -> Result<i32> {
    // Try to get existing
    let existing: Option<(i32,)> = sqlx::query_as("SELECT id FROM tags WHERE name = $1")
        .bind(name)
        .fetch_optional(pool)
        .await?;

    if let Some((id,)) = existing {
        return Ok(id);
    }

    // Insert new
    let row: (i32,) =
        sqlx::query_as("INSERT INTO tags (name, is_auto) VALUES ($1, $2) RETURNING id")
            .bind(name)
            .bind(is_auto)
            .fetch_one(pool)
            .await?;

    Ok(row.0)
}

/// Extract email address from a string like "John Doe <john@example.com>"
fn extract_email_address(from: &str) -> String {
    if let Some(start) = from.find('<') {
        if let Some(end) = from.find('>') {
            return from[start + 1..end].trim().to_string();
        }
    }
    // If no angle brackets, return as-is if it contains @
    if from.contains('@') {
        return from.trim().to_string();
    }
    String::new()
}

/// Get all tags for an email.
pub async fn get_email_tags(pool: &PgPool, email_id: i32) -> Result<Vec<String>> {
    let tags: Vec<String> = sqlx::query_scalar(
        "SELECT t.name FROM tags t JOIN email_tags et ON t.id = et.tag_id WHERE et.email_id = $1 ORDER BY t.name",
    )
    .bind(email_id)
    .fetch_all(pool)
    .await?;
    Ok(tags)
}

/// Get all auto-generated tags (for filter UI).
pub async fn list_tags(pool: &PgPool) -> Result<Vec<serde_json::Value>> {
    let rows: Vec<(i32, String, bool)> =
        sqlx::query_as("SELECT id, name, is_auto FROM tags ORDER BY name")
            .fetch_all(pool)
            .await?;

    Ok(rows
        .into_iter()
        .map(|(id, name, is_auto)| {
            serde_json::json!({
                "id": id,
                "name": name,
                "is_auto": is_auto
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_email_address() {
        assert_eq!(
            extract_email_address("John Doe <john@example.com>"),
            "john@example.com"
        );
        assert_eq!(
            extract_email_address("john@example.com"),
            "john@example.com"
        );
        assert_eq!(extract_email_address("John Doe"), "");
    }

    #[test]
    fn test_extract_email_address_with_spaces() {
        assert_eq!(
            extract_email_address("  John Doe < john@example.com >  "),
            "john@example.com"
        );
    }
}
