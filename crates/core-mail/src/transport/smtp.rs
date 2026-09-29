//! SMTP adapter: a minimal RFC 5321 client over tokio TCP — EHLO, optional
//! AUTH PLAIN, MAIL/RCPT/DATA. Targets local relays and Mailpit (dev) plus
//! self-host smarthosts on trusted networks.
//!
//! None-mode note: this adapter has no implicit none mode; configure
//! `EMAIL_TRANSPORT=none` to keep mail queued without sending.
//! Known v1 limitation (documented): STARTTLS/TLS is not implemented —
//! production internet-facing delivery should use the Postmark adapter
//! (HTTPS); use SMTP only against relays on a private network.

use crate::transport::{MailTransport, OutboundEmail, SendReceipt, TransportError};
use async_trait::async_trait;
use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

#[derive(Debug, Clone)]
pub struct SmtpTransport {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl SmtpTransport {
    /// Renders the full RFC 5322 message (headers + dot-stuffed body).
    /// Exposed for tests and so callers can log exactly what was sent.
    pub fn render_message(message: &OutboundEmail) -> String {
        let from = match &message.from_name {
            Some(name) => format!("{name} <{}>", message.from_address),
            None => message.from_address.clone(),
        };
        let mut m = String::new();
        m.push_str(&format!("From: {from}\r\n"));
        m.push_str(&format!("To: {}\r\n", message.to_addresses.join(", ")));
        if let Some(reply_to) = &message.reply_to {
            m.push_str(&format!("Reply-To: {reply_to}\r\n"));
        }
        m.push_str(&format!("Subject: {}\r\n", encode_header(&message.subject)));
        m.push_str(&format!("Date: {}\r\n", rfc2822_now()));
        m.push_str(&format!(
            "Message-ID: <{}>\r\n",
            trim_id(&message.rfc_message_id)
        ));
        if let Some(irt) = &message.in_reply_to {
            m.push_str(&format!("In-Reply-To: <{}>\r\n", trim_id(irt)));
        }
        if !message.references.is_empty() {
            let refs = message
                .references
                .iter()
                .map(|r| format!("<{r}>"))
                .collect::<Vec<_>>()
                .join(" ");
            m.push_str(&format!("References: {refs}\r\n"));
        }
        m.push_str("MIME-Version: 1.0\r\n");
        m.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        m.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
        m.push_str(&dot_stuff(&message.body_text));
        m.push_str("\r\n");
        m
    }

    fn auth_plain(&self) -> Option<String> {
        let user = self.username.as_deref()?;
        let pass = self.password.as_deref()?;
        let raw = format!("\u{0}{user}\u{0}{pass}");
        Some(base64::engine::general_purpose::STANDARD.encode(raw))
    }
}

#[async_trait]
impl MailTransport for SmtpTransport {
    fn name(&self) -> &'static str {
        "smtp"
    }

    async fn send(&self, message: &OutboundEmail) -> Result<SendReceipt, TransportError> {
        let stream = TcpStream::connect((self.host.as_str(), self.port))
            .await
            .map_err(|e| {
                TransportError::Unavailable(format!("{}:{}: {e}", self.host, self.port))
            })?;
        let mut conn = SmtpConn::new(stream);
        conn.expect_code(220, "greeting").await?;
        conn.cmd("EHLO marula.local").await?;
        conn.expect_code(250, "EHLO").await?;
        if let Some(token) = self.auth_plain() {
            conn.cmd(&format!("AUTH PLAIN {token}")).await?;
            conn.expect_code(235, "AUTH").await?;
        }
        conn.cmd(&format!("MAIL FROM:<{}>", message.from_address))
            .await?;
        conn.expect_code(250, "MAIL FROM").await?;
        for rcpt in &message.to_addresses {
            conn.cmd(&format!("RCPT TO:<{rcpt}>")).await?;
            conn.expect_code(250, "RCPT TO").await?;
        }
        conn.cmd("DATA").await?;
        conn.expect_code(354, "DATA").await?;
        conn.raw(Self::render_message(message)).await?;
        conn.cmd(".").await?;
        conn.expect_code(250, "end of DATA").await?;
        let _ = conn.cmd("QUIT").await;
        Ok(SendReceipt {
            provider_message_id: None,
        })
    }
}

/// One SMTP conversation over a single connection.
struct SmtpConn {
    reader: BufReader<tokio::io::ReadHalf<TcpStream>>,
    writer: tokio::io::WriteHalf<TcpStream>,
}

impl SmtpConn {
    fn new(stream: TcpStream) -> Self {
        let (r, w) = tokio::io::split(stream);
        Self {
            reader: BufReader::new(r),
            writer: w,
        }
    }

    /// Sends one command line (CRLF-terminated).
    async fn cmd(&mut self, line: &str) -> Result<(), TransportError> {
        self.writer
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .map_err(|e| TransportError::Unavailable(format!("write: {e}")))?;
        self.writer
            .flush()
            .await
            .map_err(|e| TransportError::Unavailable(format!("flush: {e}")))
    }

    /// Writes pre-rendered bytes verbatim (the DATA payload).
    async fn raw(&mut self, bytes: String) -> Result<(), TransportError> {
        self.writer
            .write_all(bytes.as_bytes())
            .await
            .map_err(|e| TransportError::Unavailable(format!("write: {e}")))?;
        self.writer
            .flush()
            .await
            .map_err(|e| TransportError::Unavailable(format!("flush: {e}")))
    }

    /// Reads a (possibly multiline) reply and asserts its final code.
    async fn expect_code(&mut self, want: u16, step: &str) -> Result<(), TransportError> {
        let code = loop {
            let mut line = String::new();
            self.reader
                .read_line(&mut line)
                .await
                .map_err(|e| TransportError::Unavailable(format!("read {step}: {e}")))?;
            let line = line.trim_end();
            if line.len() < 3 {
                return Err(TransportError::Rejected(format!(
                    "{step}: malformed reply {line:?}"
                )));
            }
            let code: u16 = line
                .get(..3)
                .and_then(|c| c.parse().ok())
                .ok_or_else(|| TransportError::Rejected(format!("{step}: bad code {line:?}")))?;
            // Fourth char '-' means continuation lines follow.
            if line.as_bytes().get(3) != Some(&b'-') {
                break code;
            }
        };
        if code == want {
            Ok(())
        } else {
            Err(TransportError::Rejected(format!(
                "{step}: expected {want}, got {code}"
            )))
        }
    }
}

/// RFC 5322 date (UTC, +0000).
fn rfc2822_now() -> String {
    chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S %z")
        .to_string()
}

/// Doubles leading dots per DATA framing rules.
fn dot_stuff(body: &str) -> String {
    body.split('\n')
        .map(|line| {
            if line.starts_with('.') {
                format!(".{line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Encodes a header value as `=?UTF-8?B?…?=` when it contains non-ASCII,
/// passing pure-ASCII through untouched. CRLF injection is neutralized by
/// folding newlines into spaces.
fn encode_header(value: &str) -> String {
    // Neutralize header injection: every CR/LF becomes a space.
    let folded: String = value
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    if folded.is_ascii() {
        return folded;
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(folded);
    format!("=?UTF-8?B?{b64}?=")
}

/// Stored ids are bare; the wire form wants angle brackets exactly once.
fn trim_id(raw: &str) -> &str {
    raw.trim().trim_start_matches('<').trim_end_matches('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_stuffing_only_touches_leading_dots() {
        assert_eq!(
            dot_stuff(".hidden\n..two\nplain"),
            "..hidden\n...two\nplain"
        );
    }

    #[test]
    fn ascii_headers_pass_through_and_strip_crlf_injection() {
        assert_eq!(encode_header("Hello"), "Hello");
        let folded = encode_header("bad\r\nBCC: x");
        assert!(folded.starts_with("bad"));
        assert!(folded.contains("BCC: x"));
        assert!(!folded.contains('\r'));
        assert!(!folded.contains('\n'));
    }

    #[test]
    fn utf8_subject_uses_b_encoding() {
        let enc = encode_header("Grüße");
        assert!(enc.starts_with("=?UTF-8?B?"), "{enc}");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(enc.trim_start_matches("=?UTF-8?B?").trim_end_matches("?="))
            .expect("valid b64");
        assert_eq!(String::from_utf8(decoded).expect("utf8"), "Grüße");
    }

    #[test]
    fn rendered_message_carries_threading_headers_and_from_line() {
        let m = OutboundEmail {
            from_address: "support@acme.test".into(),
            from_name: Some("Acme Support".into()),
            reply_to: Some("projects@acme.test".into()),
            to_addresses: vec!["client@example.test".into()],
            subject: "Re: Printer".into(),
            body_text: "line\n.dot".into(),
            rfc_message_id: "m1@acme.test".into(),
            in_reply_to: Some("orig@example.test".into()),
            references: vec!["orig@example.test".to_string()],
        };
        let rendered = SmtpTransport::render_message(&m);
        assert!(rendered.contains("From: Acme Support <support@acme.test>\r\n"));
        assert!(rendered.contains("Reply-To: projects@acme.test\r\n"));
        assert!(rendered.contains("In-Reply-To: <orig@example.test>\r\n"));
        assert!(rendered.contains("References: <orig@example.test>\r\n"));
        assert!(rendered.contains("\r\n..dot\r\n") || rendered.ends_with("..dot\r\n"));
    }

    #[test]
    fn no_reply_to_header_when_unset() {
        let m = OutboundEmail {
            from_address: "f@a.test".into(),
            from_name: None,
            reply_to: None,
            to_addresses: vec!["t@a.test".into()],
            subject: "s".into(),
            body_text: "b".into(),
            rfc_message_id: "m@a.test".into(),
            in_reply_to: None,
            references: vec![],
        };
        assert!(!SmtpTransport::render_message(&m).contains("Reply-To:"));
    }

    /// Scripted in-process SMTP server asserting the exact conversation.
    #[tokio::test]
    async fn smtp_conversation_happy_path() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
            let (sock, _) = listener.accept().await.unwrap();
            let (r, mut w) = tokio::io::split(sock);
            let mut r = BufReader::new(r);
            w.write_all(b"220 mailpit ready\r\n").await.unwrap();
            let mut seen = Vec::new();
            loop {
                let mut line = String::new();
                if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let l = line.trim_end().to_string();
                if l == "DATA" {
                    w.write_all(b"354 go\r\n").await.unwrap();
                    let mut body = Vec::new();
                    loop {
                        let mut buf = [0u8; 1];
                        r.read_exact(&mut buf).await.unwrap();
                        body.push(buf[0]);
                        if body.ends_with(b"\r\n.\r\n") {
                            break;
                        }
                    }
                    seen.push(String::from_utf8_lossy(&body).to_string());
                    w.write_all(b"250 queued\r\n").await.unwrap();
                } else if l.starts_with("AUTH PLAIN ") {
                    seen.push(l);
                    w.write_all(b"235 ok\r\n").await.unwrap();
                } else {
                    seen.push(l.clone());
                    match l.as_str() {
                        "QUIT" => {
                            w.write_all(b"221 bye\r\n").await.unwrap();
                            break;
                        }
                        _ => w.write_all(b"250 ok\r\n").await.unwrap(),
                    }
                }
            }
            seen
        });

        let t = SmtpTransport {
            host: addr.ip().to_string(),
            port: addr.port(),
            username: Some("u".into()),
            password: Some("p".into()),
        };
        let receipt = t
            .send(&OutboundEmail {
                from_address: "support@acme.test".into(),
                from_name: None,
                reply_to: None,
                to_addresses: vec!["a@x.test".into(), "b@x.test".into()],
                subject: "Hi".into(),
                body_text: "body text".into(),
                rfc_message_id: "id1@acme.test".into(),
                in_reply_to: None,
                references: vec![],
            })
            .await
            .expect("send succeeds");

        assert_eq!(receipt.provider_message_id, None);
        let seen = server.await.unwrap();
        assert!(seen.iter().any(|l| l.starts_with("AUTH PLAIN ")));
        assert!(seen.iter().any(|l| l == "MAIL FROM:<support@acme.test>"));
        assert!(seen.iter().any(|l| l == "RCPT TO:<a@x.test>"));
        assert!(seen.iter().any(|l| l == "RCPT TO:<b@x.test>"));
        let data = seen
            .iter()
            .find(|l| l.contains("Message-ID: <id1@acme.test>"))
            .expect("DATA payload captured");
        assert!(data.contains("body text"));
        assert!(data.ends_with("\r\n.\r\n"));
    }

    #[tokio::test]
    async fn connection_refused_maps_to_unavailable() {
        // Port 1 on localhost: nothing listens; must classify as retryable.
        let t = SmtpTransport {
            host: "127.0.0.1".into(),
            port: 1,
            username: None,
            password: None,
        };
        let err = t
            .send(&OutboundEmail {
                from_address: "f@a".into(),
                from_name: None,
                reply_to: None,
                to_addresses: vec!["t@a".into()],
                subject: "s".into(),
                body_text: "b".into(),
                rfc_message_id: "<m@a>".into(),
                in_reply_to: None,
                references: vec![],
            })
            .await;
        assert!(matches!(err, Err(TransportError::Unavailable(_))));
    }
}
