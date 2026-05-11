# Mail Archive Daemon Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** A single-user, Dockerized service that connects to IMAP accounts, downloads messages + attachments, stores permanently, and provides hybrid (keyword + semantic) search.

**Architecture:** Rust backend (axum + tokio), PostgreSQL 16 + pgvector, raw IMAP over TLS, fastembed for embeddings, vanilla JS + htmx + Bootstrap 5 frontend.

**Tech Stack:** Rust 2021, tokio, axum, sqlx, fastembed, mailparse, whatlang, Tera, Bootstrap 5, htmx, Docker.

---

## Repository Layout

```
mail-archive/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── config.rs
│   ├── db/
│   │   ├── mod.rs
│   │   └── schema.sql
│   ├── imap/
│   │   ├── mod.rs
│   │   ├── client.rs
│   │   └── sync_engine.rs
│   ├── mail/
│   │   ├── mod.rs
│   │   ├── parser.rs
│   │   └── language.rs
│   ├── embed/
│   │   └── mod.rs
│   ├── storage/
│   │   ├── mod.rs
│   │   ├── attachment.rs
│   │   └── tags.rs
│   └── api/
│       ├── mod.rs
│       ├── routes.rs
│       └── auth.rs
├── migrations/
├── templates/
│   ├── base.html
│   ├── search.html
│   ├── stats.html
│   └── partials/
│       └── result.html
├── static/
│   └── style.css
├── Dockerfile
├── .dockerignore
├── docker-compose.yml
└── README.md
```

## Database Schema (PostgreSQL + pgvector)

### settings
- id INT PRIMARY KEY
- max_parallelism INT DEFAULT 4
- default_sync_interval_seconds INT DEFAULT 300
- password_hash VARCHAR (bcrypt for web UI admin login)

### mail_accounts
- id SERIAL PRIMARY KEY
- email_address VARCHAR UNIQUE
- label VARCHAR (human-readable name)
- imap_server VARCHAR
- imap_port INT
- use_tls BOOLEAN DEFAULT true
- encrypted_password BYTEA
- encryption_nonce BYTEA
- excluded_folders JSONB DEFAULT '["Spam","Trash"]'
- grace_period_days INT DEFAULT 30
- uid_validity JSONB (folder→uidvalidity mapping)
- last_sync_at TIMESTAMP
- enabled BOOLEAN DEFAULT true

### emails
- id SERIAL PRIMARY KEY
- account_id INT FK → mail_accounts
- imap_uid BIGINT
- folder VARCHAR
- message_id VARCHAR
- received_at TIMESTAMP
- subject TEXT
- sender VARCHAR
- recipients JSONB
- body_text TEXT
- detected_lang VARCHAR(10)
- search_vector vector(384)
- fts_doc tsvector
- created_at TIMESTAMP DEFAULT NOW()
- Indexes: HNSW on search_vector, GIN on fts_doc, B-tree on (account_id, folder, imap_uid)

### attachments
- id SERIAL PRIMARY KEY
- email_id INT FK → emails ON DELETE CASCADE
- filename VARCHAR
- mime_type VARCHAR
- size_bytes BIGINT
- file_hash VARCHAR(64)
- is_inline BOOLEAN DEFAULT false

### tags
- id SERIAL PRIMARY KEY
- name VARCHAR UNIQUE
- is_auto BOOLEAN DEFAULT true

### email_tags
- email_id INT FK
- tag_id INT FK
- UNIQUE(email_id, tag_id)

### sync_errors
- id SERIAL PRIMARY KEY
- account_id INT FK
- folder VARCHAR
- error_message TEXT
- occurred_at TIMESTAMP DEFAULT NOW()

---

## Phases

See the conversation for the full task breakdown. Each phase is dispatched as a subagent task.
