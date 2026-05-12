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

CREATE TABLE mail_accounts (
    id SERIAL PRIMARY KEY,
    email_address VARCHAR NOT NULL UNIQUE,
    label VARCHAR,
    imap_server VARCHAR NOT NULL,
    imap_port INT NOT NULL DEFAULT 993,
    use_tls BOOLEAN NOT NULL DEFAULT true,
    encrypted_password BYTEA NOT NULL,
    encryption_nonce BYTEA NOT NULL,
    excluded_folders JSONB NOT NULL DEFAULT '["Spam", "Trash"]'::jsonb,
    grace_period_days INT NOT NULL DEFAULT 30,
    uid_validity JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_sync_at TIMESTAMP,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE emails (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    imap_uid BIGINT NOT NULL,
    folder VARCHAR NOT NULL,
    message_id VARCHAR,
    received_at TIMESTAMP,
    subject TEXT,
    sender VARCHAR,
    recipients JSONB,
    body_text TEXT,
    detected_lang VARCHAR(10),
    search_vector vector(384),
    fts_doc tsvector,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_emails_account_folder_uid ON emails(account_id, folder, imap_uid);
CREATE INDEX idx_emails_fts ON emails USING gin(fts_doc);
CREATE INDEX idx_emails_vector ON emails USING hnsw(search_vector vector_cosine_ops);

CREATE TABLE attachments (
    id SERIAL PRIMARY KEY,
    email_id INT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    filename VARCHAR NOT NULL,
    mime_type VARCHAR,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    file_hash VARCHAR(64) NOT NULL,
    is_inline BOOLEAN NOT NULL DEFAULT false
);

CREATE INDEX idx_attachments_email ON attachments(email_id);

CREATE TABLE tags (
    id SERIAL PRIMARY KEY,
    name VARCHAR NOT NULL UNIQUE,
    is_auto BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE email_tags (
    email_id INT NOT NULL REFERENCES emails(id) ON DELETE CASCADE,
    tag_id INT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (email_id, tag_id)
);

CREATE TABLE sync_errors (
    id SERIAL PRIMARY KEY,
    account_id INT NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    folder VARCHAR,
    error_message TEXT NOT NULL,
    occurred_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_sync_errors_account ON sync_errors(account_id);

INSERT INTO settings (id) VALUES (1) ON CONFLICT DO NOTHING;
