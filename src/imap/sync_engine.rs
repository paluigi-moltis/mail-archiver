use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use tokio::sync::Semaphore;

use super::client::ImapClient;
use crate::config::AppConfig;
use crate::crypto;

/// In-memory representation of a mail account from the DB.
#[derive(Debug, Deserialize, Serialize, Clone, FromRow)]
pub struct MailAccount {
    pub id: i32,
    pub email_address: String,
    #[allow(dead_code)]
    pub label: Option<String>,
    pub imap_server: String,
    pub imap_port: i32,
    pub use_tls: bool,
    pub encrypted_password: Vec<u8>,
    pub encryption_nonce: Vec<u8>,
    pub excluded_folders: serde_json::Value,
    pub grace_period_days: i32,
    pub uid_validity: serde_json::Value,
    pub enabled: bool,
}

impl MailAccount {
    /// Load all enabled accounts from the database.
    pub async fn load_all(pool: &PgPool) -> Result<Vec<Self>> {
        let rows: Vec<MailAccount> = sqlx::query_as(
            r#"SELECT id, email_address, label, imap_server, imap_port, use_tls,
               encrypted_password, encryption_nonce,
               excluded_folders, grace_period_days, uid_validity, enabled
               FROM mail_accounts WHERE enabled = true"#,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| anyhow!("Failed to load accounts: {e}"))?;
        Ok(rows)
    }
}

/// Sync a single account: connect, iterate folders, download new emails.
pub async fn sync_account(
    account: &MailAccount,
    pool: &PgPool,
    master_key: &str,
    semaphore: &Semaphore,
) -> Result<()> {
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|e| anyhow!("Semaphore error: {e}"))?;

    // Decrypt IMAP password
    let password = crypto::decrypt(
        master_key,
        &account.encryption_nonce,
        &account.encrypted_password,
    )
    .map_err(|e| {
        anyhow!(
            "Failed to decrypt password for {}: {e}",
            account.email_address
        )
    })?;

    // Connect to IMAP server
    let mut client = ImapClient::connect(
        &account.imap_server,
        account.imap_port as u16,
        account.use_tls,
    )
    .await
    .map_err(|e| anyhow!("Connection failed for {}: {e}", account.email_address))?;

    client
        .login(&account.email_address, &password)
        .await
        .map_err(|e| anyhow!("Login failed for {}: {e}", account.email_address))?;
    // password is now dropped from memory

    // Get list of all folders
    let all_folders = client
        .list_mailboxes("", "*")
        .await
        .map_err(|e| anyhow!("LIST failed for {}: {e}", account.email_address))?;

    // Parse excluded folders
    let excluded: HashSet<String> = if let Some(arr) = account.excluded_folders.as_array() {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
            .collect()
    } else {
        HashSet::new()
    };

    for folder in &all_folders {
        if excluded.contains(&folder.to_lowercase()) {
            continue;
        }

        if let Err(e) = sync_folder(&mut client, account, folder, pool).await {
            eprintln!(
                "Error syncing folder '{}' for account {}: {}",
                folder, account.email_address, e
            );
            // Log error to sync_errors table
            let _ = sqlx::query(
                "INSERT INTO sync_errors (account_id, folder, error_message) VALUES ($1, $2, $3)",
            )
            .bind(account.id)
            .bind(folder)
            .bind(format!("{e}"))
            .execute(pool)
            .await;
        }
    }

    client.logout().await.ok();

    // Update last_sync_at
    let _ = sqlx::query("UPDATE mail_accounts SET last_sync_at = NOW() WHERE id = $1")
        .bind(account.id)
        .execute(pool)
        .await;

    Ok(())
}

/// Sync a single folder within an account.
async fn sync_folder(
    client: &mut ImapClient,
    account: &MailAccount,
    folder: &str,
    pool: &PgPool,
) -> Result<()> {
    let (server_uidvalidity, _exists) = client
        .select_mailbox(folder)
        .await
        .map_err(|e| anyhow!("SELECT failed for {folder}: {e}"))?;

    // Check UIDVALIDITY against stored value
    let stored_uidvalidity: Option<i64> = account
        .uid_validity
        .as_object()
        .and_then(|map| map.get(folder))
        .and_then(|v| v.as_i64());

    if let Some(stored) = stored_uidvalidity {
        if stored != server_uidvalidity {
            // UIDVALIDITY changed — wipe this folder's emails
            eprintln!(
                "UIDVALIDITY changed for {}:{} ({} -> {}), wiping...",
                account.email_address, folder, stored, server_uidvalidity
            );
            sqlx::query("DELETE FROM emails WHERE account_id = $1 AND folder = $2")
                .bind(account.id)
                .bind(folder)
                .execute(pool)
                .await?;
        }
    }

    // Update UIDVALIDITY in DB
    let mut uid_validity_map = account
        .uid_validity
        .as_object()
        .cloned()
        .unwrap_or_default();
    uid_validity_map.insert(folder.to_string(), serde_json::json!(server_uidvalidity));
    let new_uid_validity = serde_json::to_value(uid_validity_map)?;
    sqlx::query("UPDATE mail_accounts SET uid_validity = $1 WHERE id = $2")
        .bind(&new_uid_validity)
        .bind(account.id)
        .execute(pool)
        .await?;

    // Get all server UIDs
    let server_uids: HashSet<i64> = client.uid_search_all().await?.into_iter().collect();

    // Get max stored UID for this account+folder to find new ones
    let max_uid: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(imap_uid) FROM emails WHERE account_id = $1 AND folder = $2",
    )
    .bind(account.id)
    .bind(folder)
    .fetch_optional(pool)
    .await?;

    // Download new emails (UIDs greater than max stored)
    let new_uids: Vec<i64> = server_uids
        .iter()
        .filter(|uid| max_uid.is_none_or(|max| **uid > max))
        .copied()
        .collect();

    for uid in &new_uids {
        match client.uid_fetch_raw(*uid).await {
            Ok(raw_email) => {
                if let Err(e) = store_raw_email(pool, account.id, *uid, folder, &raw_email).await {
                    eprintln!(
                        "Error storing UID {} from {}: {}",
                        uid, account.email_address, e
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "Error fetching UID {} from {}: {}",
                    uid, account.email_address, e
                );
            }
        }
    }

    // Grace period deletion
    if account.grace_period_days > 0 {
        grace_period_delete(pool, account, folder, &server_uids).await?;
    }

    Ok(())
}

/// Insert a raw email into the database (minimal placeholder — Phase 4 will expand).
async fn store_raw_email(
    pool: &PgPool,
    account_id: i32,
    imap_uid: i64,
    folder: &str,
    _raw_email: &[u8],
) -> Result<()> {
    // Minimal insertion — Phase 4 will expand with full parsing, embedding, tagging
    let message_id = format!("temp-{}-{}", account_id, imap_uid);

    sqlx::query(
        "INSERT INTO emails (account_id, imap_uid, folder, message_id, body_text) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(account_id)
    .bind(imap_uid)
    .bind(folder)
    .bind(&message_id)
    .bind("Not yet parsed")
    .execute(pool)
    .await
    .map_err(|e| anyhow!("DB insert error for UID {imap_uid}: {e}"))?;

    Ok(())
}

/// Delete emails that are past the grace period AND missing from the server.
async fn grace_period_delete(
    pool: &PgPool,
    account: &MailAccount,
    folder: &str,
    server_uids: &HashSet<i64>,
) -> Result<()> {
    let cutoff = Utc::now() - chrono::Duration::days(account.grace_period_days as i64);

    let db_uids: Vec<i64> = sqlx::query_scalar(
        "SELECT imap_uid FROM emails WHERE account_id = $1 AND folder = $2 AND received_at < $3",
    )
    .bind(account.id)
    .bind(folder)
    .bind(cutoff)
    .fetch_all(pool)
    .await?;

    let mut deleted_count = 0i64;
    for uid in db_uids {
        if !server_uids.contains(&uid) {
            sqlx::query(
                "DELETE FROM emails WHERE account_id = $1 AND folder = $2 AND imap_uid = $3",
            )
            .bind(account.id)
            .bind(folder)
            .bind(uid)
            .execute(pool)
            .await?;
            deleted_count += 1;
        }
    }

    if deleted_count > 0 {
        eprintln!(
            "Grace-deleted {} emails from {}:{} (past {}-day grace period)",
            deleted_count, account.email_address, folder, account.grace_period_days
        );
    }

    Ok(())
}

/// Main sync loop — runs periodically in the background.
/// This function never returns (it loops forever).
pub async fn run_sync_loop(pool: PgPool, _config: AppConfig, master_key: String) {
    let interval_secs = {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT default_sync_interval_seconds FROM settings WHERE id = 1")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();
        row.map(|(s,)| s as u64).unwrap_or(300)
    };

    let max_parallelism = {
        let row: Option<(i32,)> =
            sqlx::query_as("SELECT max_parallelism FROM settings WHERE id = 1")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();
        row.map(|(s,)| s as usize).unwrap_or(4)
    };

    let semaphore = Arc::new(Semaphore::new(max_parallelism));
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

    loop {
        interval.tick().await;

        let accounts = match MailAccount::load_all(&pool).await {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Error loading accounts: {}", e);
                continue;
            }
        };

        if accounts.is_empty() {
            continue;
        }

        println!("Starting sync for {} accounts...", accounts.len());

        for account in &accounts {
            let pool = pool.clone();
            let master_key = master_key.clone();
            let semaphore = semaphore.clone();
            let account = account.clone();

            tokio::spawn(async move {
                if let Err(e) = sync_account(&account, &pool, &master_key, &semaphore).await {
                    eprintln!("Sync error for {}: {}", account.email_address, e);
                }
            });
        }

        println!("Sync tasks dispatched, waiting for next interval...");
    }
}
