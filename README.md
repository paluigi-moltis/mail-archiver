# Mail Archive Daemon

**Self-hosted, Dockerized, single-user email archiving solution** — connect to multiple IMAP accounts, download emails and attachments, store permanently, and search with hybrid keyword + semantic search.

![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)
![PostgreSQL](https://img.shields.io/badge/PostgreSQL-16+-336791.svg)
![Docker](https://img.shields.io/badge/Docker-ready-2496ED?logo=docker&logoColor=white)
![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)

## Architecture

```
┌──────────────┐     ┌──────────────┐     ┌───────────────┐
│  IMAP        │────▶│  Sync Engine │────▶│  PostgreSQL   │
│  Accounts    │     │  (Tokio bg)  │     │  + pgvector   │
└──────────────┘     └──────┬───────┘     └───────┬───────┘
                            │                     │
                     ┌──────┴───────┐     ┌──────┴───────┐
                     │  mailparse   │     │  FTS + Vector │
                     │  + fastembed │     │  Search (RRF) │
                     └──────────────┘     └──────┬───────┘
                                                  │
                                           ┌──────┴───────┐
                                           │  Web UI      │
                                           │  Axum + Tera │
                                           │  Bootstrap 5 │
                                           └──────────────┘
```

## Features

- **Multi-account IMAP sync** — connect to any number of IMAP servers (TLS or plaintext)
- **Grace-period deletion** — emails deleted from the server are removed from the archive only after a configurable grace period
- **Content-addressable attachments** — deduplicated via SHA-256, stored on disk
- **Language-aware FTS** — PostgreSQL full-text search with automatic language detection
- **Semantic search** — powered by fastembed (all-MiniLM-L6-v2, runs on CPU)
- **Auto-tagging** — `folder:`, `from:`, `has:attachment` tags generated automatically
- **Zero-build frontend** — Bootstrap 5 + htmx, no Node.js required
- **Dockerized** — multi-stage Dockerfile, single `docker compose up` to deploy

## Quick Start

### 1. Configure environment

```bash
cp .env.example .env
```

Edit `.env`:

```env
ARCHIVE_MASTER_KEY=<64-char hex string>  # Generate with: openssl rand -hex 32
JWT_SECRET=<random secret>
DB_PASSWORD=mail_archive_dev
```

### 2. Start with Docker Compose

```bash
docker compose up -d
```

This starts:
- **PostgreSQL 16** with pgvector extension
- **Mail Archive** app on port 8000

### 3. Open the UI

Navigate to [http://localhost:8000](http://localhost:8000).

On first login, any password will be accepted and set as the admin password.

### 4. Add IMAP accounts

Use the API or the Accounts page to add your IMAP accounts:

```bash
curl -X POST http://localhost:8000/api/accounts \
  -H "Content-Type: application/json" \
  -d '{
    "email_address": "you@gmail.com",
    "label": "Gmail",
    "imap_server": "imap.gmail.com",
    "imap_port": 993,
    "use_tls": true,
    "password": "your-app-password",
    "grace_period_days": 30
  }'
```

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/` | Search page |
| `GET` | `/stats` | Stats dashboard |
| `GET` | `/health` | Health check |
| `POST` | `/api/auth/login` | Admin login |
| `GET` | `/api/accounts` | List IMAP accounts |
| `POST` | `/api/accounts` | Add IMAP account |
| `DELETE` | `/api/accounts/{id}` | Remove account |
| `GET` | `/api/settings` | Get settings |
| `PUT` | `/api/settings` | Update settings |
| `GET` | `/api/search?q=...&page=1&tags=...` | Search emails |
| `GET` | `/api/attachments/{hash}` | Download attachment |
| `GET` | `/api/stats` | Storage statistics |
| `GET` | `/api/tags` | List all tags |

## Tech Stack

| Component | Technology |
|-----------|-----------|
| Backend | Rust 2021, Axum, Tokio |
| Database | PostgreSQL 16 + pgvector |
| IMAP | Raw TLS via tokio-rustls |
| Mail parsing | mailparse |
| Embeddings | fastembed (all-MiniLM-L6-v2, CPU) |
| Language detection | whatlang |
| Cryptography | AES-256-GCM (argon2 for auth) |
| Frontend | Tera + Bootstrap 5 + htmx |

## Project Structure

```
mail-archive/
├── Cargo.toml
├── Dockerfile
├── docker-compose.yml
├── migrations/
│   └── 20240101000000_create_schema.sql
├── templates/
│   ├── base.html
│   ├── search.html
│   └── stats.html
├── static/
│   └── style.css
└── src/
    ├── main.rs
    ├── config.rs
    ├── crypto.rs
    ├── db/
    │   └── mod.rs
    ├── imap/
    │   ├── mod.rs
    │   ├── client.rs
    │   ├── split.rs
    │   └── sync_engine.rs
    ├── mail/
    │   ├── mod.rs
    │   ├── parser.rs
    │   └── language.rs
    ├── embed/
    │   └── mod.rs
    ├── storage/
    │   ├── mod.rs
    │   ├── attachment.rs
    │   └── tags.rs
    └── api/
        ├── mod.rs
        ├── auth.rs
        └── routes.rs
```

## License

MIT
