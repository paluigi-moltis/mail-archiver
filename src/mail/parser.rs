#![allow(dead_code)]

use anyhow::{anyhow, Result};
use mailparse::{parse_mail, MailHeaderMap, ParsedMail};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ParsedEmail {
    pub subject: String,
    pub from: String,
    pub to: String,
    pub cc: Option<String>,
    pub date: Option<String>,
    pub message_id: Option<String>,
    pub body_text: String,
    pub attachments: Vec<AttachmentInfo>,
    pub recipients_json: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AttachmentInfo {
    pub filename: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub is_inline: bool,
    pub file_hash: String,
}

impl ParsedEmail {
    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        let parsed = parse_mail(raw).map_err(|e| anyhow!("Failed to parse email: {}", e))?;

        let headers = &parsed.headers;

        let subject = headers.get_first_value("Subject").unwrap_or_default();
        let from = headers.get_first_value("From").unwrap_or_default();
        let to = headers.get_first_value("To").unwrap_or_default();
        let cc = headers.get_first_value("Cc");
        let date = headers.get_first_value("Date");
        let message_id = headers
            .get_first_value("Message-ID")
            .or_else(|| headers.get_first_value("Message-Id"));

        // Extract body text
        let body_text = extract_body_text(&parsed)?;

        // Extract attachments
        let attachments = extract_attachments(&parsed)?;

        // Build recipients JSON
        let mut recipients = serde_json::Map::new();
        if !to.is_empty() {
            recipients.insert("to".to_string(), serde_json::Value::String(to.clone()));
        }
        if let Some(cc_val) = &cc {
            if !cc_val.is_empty() {
                recipients.insert("cc".to_string(), serde_json::Value::String(cc_val.clone()));
            }
        }
        let recipients_json = serde_json::Value::Object(recipients);

        Ok(ParsedEmail {
            subject,
            from,
            to,
            cc,
            date,
            message_id,
            body_text,
            attachments,
            recipients_json,
        })
    }

    /// Get first N words of body for embedding
    pub fn embed_text(&self, max_words: usize) -> String {
        let combined = if self.subject.is_empty() {
            self.body_text.clone()
        } else {
            format!("{} {}", self.subject, self.body_text)
        };

        combined
            .split_whitespace()
            .take(max_words)
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn extract_body_text(parsed: &ParsedMail<'_>) -> Result<String> {
    // Try to find text/plain part first
    if let Some(body) = find_text_part(parsed, "text/plain") {
        return Ok(body);
    }

    // Fall back to text/html and strip tags
    if let Some(html) = find_text_part(parsed, "text/html") {
        return Ok(strip_html(&html));
    }

    // Fall back to the main body
    let raw_body = parsed
        .get_body_raw()
        .map_err(|e| anyhow!("Failed to get body: {}", e))?;
    Ok(String::from_utf8_lossy(&raw_body).into_owned())
}

fn find_text_part<'a>(parsed: &'a ParsedMail<'a>, mime_type: &str) -> Option<String> {
    if parsed.ctype.mimetype.eq_ignore_ascii_case(mime_type) {
        return parsed.get_body().ok();
    }

    // Search subparts
    for subpart in &parsed.subparts {
        if let Some(body) = find_text_part(subpart, mime_type) {
            return Some(body);
        }
    }

    None
}

fn strip_html(html: &str) -> String {
    let document = Html::parse_document(html);

    // Collect all text, skipping script/style elements
    let selector = Selector::parse("script, style").unwrap();
    let skip_nodes: std::collections::HashSet<_> = document
        .select(&selector)
        .flat_map(|el| el.descendants().map(|n| n.id()))
        .collect();

    let text: Vec<&str> = document
        .root_element()
        .descendants()
        .filter(|node| !skip_nodes.contains(&node.id()))
        .filter_map(|node| {
            let value = node.value();
            if let scraper::node::Node::Text(text_node) = value {
                Some(text_node.as_ref())
            } else {
                None
            }
        })
        .collect();

    let joined = text.join(" ");

    // Clean up whitespace
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_attachments(parsed: &ParsedMail<'_>) -> Result<Vec<AttachmentInfo>> {
    let mut attachments = Vec::new();
    extract_attachments_recursive(parsed, &mut attachments)?;
    Ok(attachments)
}

fn extract_attachments_recursive(
    parsed: &ParsedMail<'_>,
    attachments: &mut Vec<AttachmentInfo>,
) -> Result<()> {
    let content_disposition = parsed
        .headers
        .get_first_value("Content-Disposition")
        .unwrap_or_default();

    let is_inline = content_disposition.to_lowercase().contains("inline")
        || parsed.ctype.mimetype.starts_with("image/");

    let is_attachment = content_disposition.to_lowercase().contains("attachment") || is_inline;

    if is_attachment && !parsed.ctype.mimetype.starts_with("text/") {
        let filename = parsed
            .headers
            .get_first_value("Content-Disposition")
            .and_then(|cd| {
                // Parse filename="..."
                if let Some(start) = cd.find("filename=") {
                    let rest = &cd[start + 9..];
                    let trimmed = rest.trim_matches('"');
                    Some(trimmed.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unnamed_attachment".to_string());

        if let Ok(data) = parsed.get_body_raw() {
            if !data.is_empty() {
                let hash = compute_hash(&data);
                attachments.push(AttachmentInfo {
                    filename,
                    mime_type: parsed.ctype.mimetype.clone(),
                    data,
                    is_inline,
                    file_hash: hash,
                });
            }
        }
    }

    // Recurse into subparts
    for subpart in &parsed.subparts {
        extract_attachments_recursive(subpart, attachments)?;
    }

    Ok(())
}

pub fn compute_hash(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_email() {
        let raw = b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\nMessage-ID: <test@example.com>\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\n\r\nHello World\r\n";
        let email = ParsedEmail::from_bytes(raw).unwrap();
        assert_eq!(email.subject, "Test");
        assert_eq!(email.from, "sender@example.com");
        assert_eq!(email.to, "recipient@example.com");
        assert_eq!(email.body_text.trim(), "Hello World");
        assert_eq!(email.message_id, Some("<test@example.com>".to_string()));
        assert!(email.attachments.is_empty());
    }

    #[test]
    fn test_embed_text_truncation() {
        let email = ParsedEmail {
            subject: "Test Subject".to_string(),
            from: String::new(),
            to: String::new(),
            cc: None,
            date: None,
            message_id: None,
            body_text: "one two three four five six seven eight nine ten".to_string(),
            attachments: Vec::new(),
            recipients_json: serde_json::Value::Null,
        };
        // Subject (2 words) + body words = 5 total words
        let result = email.embed_text(5);
        assert_eq!(result, "Test Subject one two three");
    }

    #[test]
    fn test_compute_hash() {
        let hash1 = compute_hash(b"hello");
        let hash2 = compute_hash(b"hello");
        let hash3 = compute_hash(b"world");
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
        assert_eq!(hash1.len(), 64); // SHA-256 hex
    }

    #[test]
    fn test_strip_html() {
        let html = "<html><body><h1>Title</h1><p>Hello <b>world</b></p><script>var x=1;</script></body></html>";
        let text = strip_html(html);
        assert!(text.contains("Title"));
        assert!(text.contains("Hello"));
        assert!(text.contains("world"));
        // script content should not appear
        assert!(!text.contains("var x=1"));
    }

    #[test]
    fn test_extract_email_with_cc() {
        let raw = b"From: sender@example.com\r\nTo: recipient@example.com\r\nCc: copy@example.com\r\nSubject: CC Test\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\n\r\nBody text\r\n";
        let email = ParsedEmail::from_bytes(raw).unwrap();
        assert_eq!(email.cc, Some("copy@example.com".to_string()));
        assert_eq!(email.recipients_json["cc"], "copy@example.com");
    }
}
