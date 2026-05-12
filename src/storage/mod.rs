#![allow(dead_code)]

pub mod attachment;
pub mod tags;

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::embed::{embed, Embedder};
use crate::mail::language::{detect_language, lang_to_ts_config};
use crate::mail::parser::ParsedEmail;

/// Full email processing pipeline:
/// 1. Parse raw email bytes
/// 2. Insert email row
/// 3. Save attachments
/// 4. Generate embedding vector
/// 5. Build FTS document
/// 6. Auto-tag
#[allow(clippy::too_many_arguments)]
pub async fn process_and_store_email(
    pool: &PgPool,
    account_id: i32,
    imap_uid: i64,
    folder: &str,
    raw_email: &[u8],
    embedder: &Embedder,
    _master_key: &str,
    data_dir: &str,
) -> Result<i32> {
    // 1. Parse
    let parsed = ParsedEmail::from_bytes(raw_email)?;

    // 2. Parse date
    let received_at = parsed
        .date
        .as_ref()
        .and_then(|d| chrono::DateTime::parse_from_rfc2822(d).ok())
        .map(|dt: DateTime<chrono::FixedOffset>| dt.with_timezone(&Utc));

    // 3. Detect language
    let detected_lang = detect_language(&parsed.body_text);
    let lang_code = detected_lang.as_deref().unwrap_or("en");
    let ts_config = lang_to_ts_config(lang_code);

    // 4. Insert email row (without vector/fts first)
    let email_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO emails (account_id, imap_uid, folder, message_id, received_at, subject, sender, recipients, body_text, detected_lang)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
           RETURNING id"#,
    )
    .bind(account_id)
    .bind(imap_uid)
    .bind(folder)
    .bind(&parsed.message_id)
    .bind(received_at)
    .bind(&parsed.subject)
    .bind(&parsed.from)
    .bind(&parsed.recipients_json)
    .bind(&parsed.body_text)
    .bind(lang_code)
    .fetch_one(pool)
    .await?;

    // 5. Save attachments
    let has_attachment = !parsed.attachments.is_empty();
    for att in &parsed.attachments {
        if let Err(e) = attachment::save_attachment(pool, email_id, att, data_dir).await {
            eprintln!("Error saving attachment '{}': {}", att.filename, e);
        }
    }

    // 6. Generate embedding
    let embed_text = parsed.embed_text(500);
    let search_vector = if !embed_text.is_empty() {
        embed(&embed_text, embedder).await.ok()
    } else {
        None
    };

    // 7. Update with FTS and vector
    let fts_input = format!("{} {}", parsed.subject, parsed.body_text);

    if let Some(vector) = search_vector {
        let vector_str = format!(
            "[{}]",
            vector
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        sqlx::query(
            "UPDATE emails SET search_vector = $1::vector, fts_doc = to_tsvector($2, $3) WHERE id = $4",
        )
        .bind(&vector_str)
        .bind(ts_config)
        .bind(&fts_input)
        .bind(email_id)
        .execute(pool)
        .await?;
    } else {
        // Just FTS, no vector
        sqlx::query("UPDATE emails SET fts_doc = to_tsvector($1, $2) WHERE id = $3")
            .bind(ts_config)
            .bind(&fts_input)
            .bind(email_id)
        .execute(pool)
        .await?;
    }

    // 8. Auto-tag
    if let Err(e) = tags::auto_tag_email(pool, email_id, folder, &parsed.from, has_attachment).await
    {
        eprintln!("Error auto-tagging email {}: {}", email_id, e);
    }

    Ok(email_id)
}
