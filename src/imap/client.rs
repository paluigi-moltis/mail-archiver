use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use rustls::pki_types::ServerName;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use super::split::{self, ReadHalf, WriteHalf};

static TAG_COUNTER: AtomicU32 = AtomicU32::new(1);

fn next_tag() -> String {
    let tag = TAG_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("A{:03}", tag)
}

#[derive(Debug)]
pub struct ImapResponse {
    pub status: String,
    pub text: String,
    pub data: Vec<String>,
}

/// A stateful IMAP client that maintains a single TCP/TLS connection.
pub struct ImapClient {
    writer: Option<BufWriter<WriteHalf>>,
    reader: Option<BufReader<ReadHalf>>,
}

impl ImapClient {
    /// Connect to an IMAP server. If `use_tls` is true, wraps the connection
    /// in TLS immediately (IMAPS, port 993). Otherwise connects in plaintext.
    pub async fn connect(host: &str, port: u16, use_tls: bool) -> Result<Self> {
        let tcp = TcpStream::connect((host, port)).await?;

        if use_tls {
            // Direct TLS (IMAPS, port 993) — wrap in TLS before reading greeting
            let mut root_store = rustls::RootCertStore::empty();
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

            let config = rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth();

            let connector = TlsConnector::from(Arc::new(config));
            let server_name = ServerName::try_from(host)?.to_owned();
            let tls_stream = connector.connect(server_name, tcp).await?;

            let (read_half, write_half) = split::split_tls(tls_stream);
            let mut reader = BufReader::new(read_half);
            let writer = BufWriter::new(write_half);

            // Read TLS server greeting
            let mut greeting = String::new();
            reader.read_line(&mut greeting).await?;
            if !greeting.starts_with("* OK") && !greeting.starts_with("* PREAUTH") {
                return Err(anyhow!("Unexpected IMAP greeting: {}", greeting.trim()));
            }

            Ok(Self {
                writer: Some(writer),
                reader: Some(reader),
            })
        } else {
            // Plaintext (port 143)
            let (read_half, write_half) = split::split_tcp(tcp);
            let mut reader = BufReader::new(read_half);
            let writer = BufWriter::new(write_half);

            // Read plaintext server greeting
            let mut greeting = String::new();
            reader.read_line(&mut greeting).await?;
            if !greeting.starts_with("* OK") && !greeting.starts_with("* PREAUTH") {
                return Err(anyhow!("Unexpected IMAP greeting: {}", greeting.trim()));
            }

            Ok(Self {
                writer: Some(writer),
                reader: Some(reader),
            })
        }
    }

    /// Send a tagged IMAP command and collect the full response.
    pub async fn command(&mut self, cmd: &str) -> Result<ImapResponse> {
        let tag = next_tag();
        let full_cmd = format!("{} {}\r\n", tag, cmd);

        let stream = self
            .writer
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected"))?;
        stream
            .write_all(full_cmd.as_bytes())
            .await
            .map_err(|e| anyhow!("Write error: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| anyhow!("Flush error: {e}"))?;

        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let mut data = Vec::new();
        let status_line;
        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .await
                .map_err(|e| anyhow!("Read error: {e}"))?;

            // Handle literal string continuation: line ends with {N}
            if let Some(size) = extract_literal_size(&line) {
                // Read the literal bytes
                let mut literal_buf = vec![0u8; size];
                reader
                    .read_exact(&mut literal_buf)
                    .await
                    .map_err(|e| anyhow!("Literal read error: {e}"))?;
                // Read the trailing CRLF
                let mut crlf = [0u8; 2];
                reader
                    .read_exact(&mut crlf)
                    .await
                    .map_err(|e| anyhow!("CRLF read error: {e}"))?;
                data.push(String::from_utf8_lossy(&literal_buf).to_string());
                continue;
            }

            let trimmed = line.trim().to_string();

            if trimmed.starts_with(&format!("{} ", tag)) {
                // Tagged completion line
                status_line = trimmed;
                break;
            } else {
                data.push(trimmed);
            }
        }

        // Parse status: "A001 OK ..." or "A001 NO ..."
        let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
        let status = parts.get(1).unwrap_or(&"").to_string();
        let text = parts.get(2).unwrap_or(&"").to_string();

        Ok(ImapResponse { status, text, data })
    }

    /// LOGIN command
    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        let resp = self
            .command(&format!("LOGIN {} {}", quote(username), quote(password)))
            .await?;
        if resp.status != "OK" {
            return Err(anyhow!("LOGIN failed: {}", resp.text));
        }
        Ok(())
    }

    /// SELECT a mailbox. Returns (UIDVALIDITY, EXISTS count).
    pub async fn select_mailbox(&mut self, folder: &str) -> Result<(i64, i64)> {
        let resp = self.command(&format!("SELECT {}", quote(folder))).await?;
        if resp.status != "OK" {
            return Err(anyhow!("SELECT failed: {}", resp.text));
        }

        let mut uidvalidity: i64 = 0;
        let mut exists: i64 = 0;

        for line in &resp.data {
            let upper = line.to_uppercase();
            if upper.contains("UIDVALIDITY") {
                if let Some(pos) = upper.find("UIDVALIDITY") {
                    let after = &line[pos + "UIDVALIDITY".len()..];
                    let num_str: String =
                        after.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num_str.parse::<i64>() {
                        uidvalidity = n;
                    }
                }
            }
            if upper.contains("EXISTS") {
                if let Some(pos) = upper.find("EXISTS") {
                    let before = &line[..pos];
                    let num_str: String = before
                        .chars()
                        .rev()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    if let Ok(n) = num_str.parse::<i64>() {
                        exists = n;
                    }
                }
            }
        }

        Ok((uidvalidity, exists))
    }

    /// UID SEARCH ALL — returns all UIDs in the currently selected mailbox.
    pub async fn uid_search_all(&mut self) -> Result<Vec<i64>> {
        let resp = self.command("UID SEARCH ALL").await?;
        if resp.status != "OK" && !resp.data.iter().any(|l| l.contains("SEARCH")) {
            return Err(anyhow!("UID SEARCH failed: {}", resp.text));
        }

        let mut uids = Vec::new();
        for line in &resp.data {
            if line.contains("SEARCH") {
                // "* SEARCH 1 2 3 4 5"
                let parts: Vec<&str> = line.split_whitespace().collect();
                for part in parts.iter().skip(1) {
                    if let Ok(uid) = part.parse::<i64>() {
                        uids.push(uid);
                    }
                }
            }
        }

        Ok(uids)
    }

    /// UID FETCH — fetch a specific message by UID, returns raw RFC 822 bytes.
    /// This method sends the command and reads the response directly to properly
    /// handle literal (binary) data in the email body.
    pub async fn uid_fetch_raw(&mut self, uid: i64) -> Result<Vec<u8>> {
        let tag = next_tag();
        let cmd = format!("{} UID FETCH {} (BODY.PEEK[])\r\n", tag, uid);

        let stream = self
            .writer
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected"))?;
        stream
            .write_all(cmd.as_bytes())
            .await
            .map_err(|e| anyhow!("Write error: {e}"))?;
        stream
            .flush()
            .await
            .map_err(|e| anyhow!("Flush error: {e}"))?;

        let reader = self
            .reader
            .as_mut()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let mut raw_data = Vec::new();
        let mut found_body = false;

        loop {
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .await
                .map_err(|e| anyhow!("Read error: {e}"))?;

            // Check for literal marker — this is where the body lives
            if let Some(size) = extract_literal_size(&line) {
                found_body = true;
                raw_data.resize(size, 0);
                reader
                    .read_exact(&mut raw_data)
                    .await
                    .map_err(|e| anyhow!("Body read error for UID {uid}: {e}"))?;
                // Read trailing CRLF after literal
                let mut crlf = [0u8; 2];
                reader
                    .read_exact(&mut crlf)
                    .await
                    .map_err(|e| anyhow!("CRLF read error: {e}"))?;
            }

            // Check for tagged completion
            if line.trim().starts_with(&format!("{} ", tag)) {
                break;
            }
        }

        if !found_body {
            return Err(anyhow!("No body in FETCH response for UID {}", uid));
        }

        Ok(raw_data)
    }

    /// LIST all mailboxes matching a pattern.
    pub async fn list_mailboxes(&mut self, reference: &str, pattern: &str) -> Result<Vec<String>> {
        let resp = self
            .command(&format!("LIST {} {}", quote(reference), quote(pattern)))
            .await?;
        let mut folders = Vec::new();
        for line in &resp.data {
            if line.to_uppercase().starts_with("* LIST") {
                // * LIST (\HasNoChildren) "/" "INBOX"
                // Extract the last quoted string (the folder name)
                if let Some(last_quote) = line.rfind('"') {
                    if let Some(second_last) = line[..last_quote].rfind('"') {
                        let folder = &line[second_last + 1..last_quote];
                        folders.push(folder.to_string());
                    }
                }
            }
        }
        Ok(folders)
    }

    /// LOGOUT and close connection.
    pub async fn logout(&mut self) -> Result<()> {
        let _ = self.command("LOGOUT").await;
        self.writer = None;
        self.reader = None;
        Ok(())
    }
}

/// Extract the literal size from an IMAP response line like `BODY[] {12345}`.
/// Returns None if no literal marker is found.
fn extract_literal_size(line: &str) -> Option<usize> {
    if let Some(start) = line.rfind('{') {
        if let Some(end) = line[start..].find('}') {
            let inner = &line[start + 1..start + end];
            return inner.parse::<usize>().ok();
        }
    }
    None
}

/// Quote an IMAP string argument (wraps in double quotes, escapes internal quotes).
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_tag_increments() {
        TAG_COUNTER.store(1, Ordering::Relaxed);
        assert_eq!(next_tag(), "A001");
        assert_eq!(next_tag(), "A002");
        assert_eq!(next_tag(), "A003");
    }

    #[test]
    fn test_extract_literal_size() {
        assert_eq!(extract_literal_size("BODY[] {12345}"), Some(12345));
        assert_eq!(
            extract_literal_size("* 1 FETCH (UID 5 BODY[] {42}"),
            Some(42)
        );
        assert_eq!(extract_literal_size("* 1 FETCH (UID 5)"), None);
        assert_eq!(extract_literal_size("no braces here"), None);
    }

    #[test]
    fn test_quote() {
        assert_eq!(quote("INBOX"), "\"INBOX\"");
        assert_eq!(quote("folder \"name\""), "\"folder \\\"name\\\"\"");
    }
}
