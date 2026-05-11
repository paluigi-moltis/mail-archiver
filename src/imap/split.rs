//! Type aliases for split read/write halves of a TLS or TCP stream.
//!
//! IMAP needs separate read and write handles over the same connection.
//! We use `tokio::io::split` which yields concrete halves. Since TLS and
//! plain TCP have different split types, we box them behind trait objects
//! so `ImapClient` doesn't need to be generic.

use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

/// The read half of a connection (works for both TLS and plaintext).
pub type ReadHalf = Box<dyn tokio::io::AsyncRead + Unpin + Send>;

/// The write half of a connection (works for both TLS and plaintext).
pub type WriteHalf = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

/// Split a TLS stream into separate read and write halves.
pub fn split_tls(stream: TlsStream<TcpStream>) -> (ReadHalf, WriteHalf) {
    let (r, w) = tokio::io::split(stream);
    (Box::new(r), Box::new(w))
}

/// Split a plain TCP stream into separate read and write halves.
pub fn split_tcp(stream: TcpStream) -> (ReadHalf, WriteHalf) {
    let (r, w) = tokio::io::split(stream);
    (Box::new(r), Box::new(w))
}
