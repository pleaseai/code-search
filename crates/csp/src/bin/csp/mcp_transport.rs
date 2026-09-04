//! MCP stdio transport with proper `Content-Length` framing.
//!
//! `rmcp`'s built-in `stdio()` transport is newline-delimited. MCP clients
//! expect LSP-style `Content-Length: N\r\n\r\n` framing for stdio servers.
//! This module provides a drop-in replacement that speaks that dialect.

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use rmcp::service::{RxJsonRpcMessage, ServiceRole, TxJsonRpcMessage};
use rmcp::transport::Transport;
use rmcp::ErrorData;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter, Stdin, Stdout};
use tokio::sync::Mutex;

/// Maximum message size accepted by the transport before it drops the message
/// and keeps reading. 8 MiB matches the limit used by the reference MCP
/// stdio transports.
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Maximum length of a single header line. 8 KiB is well above any reasonable
/// MCP/HTTP-style header and bounds a client-controlled allocation.
const MAX_HEADER_LINE_BYTES: usize = 8 * 1024;

/// Maximum size of the entire header block. 64 KiB is generous for the single
/// `Content-Length` header MCP uses while capping total client-controlled
/// header memory.
const MAX_HEADER_BLOCK_BYTES: usize = 64 * 1024;

/// Stdio transport that frames MCP messages with `Content-Length` headers.
pub struct ContentLengthTransport<R: ServiceRole> {
    /// Buffered reader over `stdin`.
    read: BufReader<Stdin>,
    /// Buffered writer over `stdout`, shared with all in-flight `send` tasks.
    write: Arc<Mutex<BufWriter<Stdout>>>,
    _role: PhantomData<R>,
}

impl<R: ServiceRole> ContentLengthTransport<R> {
    /// Create a new content-length-framed transport from `stdin` and `stdout`.
    pub fn stdio() -> Self {
        Self {
            read: BufReader::new(tokio::io::stdin()),
            write: Arc::new(Mutex::new(BufWriter::new(tokio::io::stdout()))),
            _role: PhantomData,
        }
    }
}

impl<R: ServiceRole> Transport<R> for ContentLengthTransport<R> {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<R>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let lock = self.write.clone();
        async move {
            let body = serde_json::to_vec(&item)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let header = format!("Content-Length: {}\r\n\r\n", body.len());

            let mut write = lock.lock().await;
            write.write_all(header.as_bytes()).await?;
            write.write_all(&body).await?;
            write.flush().await?;
            Ok(())
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<R>> {
        loop {
            let content_length = match read_content_length(&mut self.read).await {
                Some(len) => len,
                None => return None,
            };

            if content_length > MAX_MESSAGE_BYTES {
                eprintln!(
                    "csp mcp: dropping oversized MCP message: {} bytes (max {})",
                    content_length, MAX_MESSAGE_BYTES
                );
                if skip_exact(&mut self.read, content_length).await.is_err() {
                    return None;
                }
                continue;
            }

            let mut body = vec![0u8; content_length];
            if self.read.read_exact(&mut body).await.is_err() {
                return None;
            }

            match serde_json::from_slice(&body) {
                Ok(message) => return Some(message),
                Err(e) => {
                    eprintln!("csp mcp: failed to parse MCP message body: {e}");
                    let response = TxJsonRpcMessage::<R>::error(
                        ErrorData::parse_error("Parse error", None),
                        None,
                    );
                    if self.send(response).await.is_err() {
                        return None;
                    }
                }
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let mut write = self.write.lock().await;
        write.flush().await?;
        write.shutdown().await?;
        Ok(())
    }
}

/// Read the `Content-Length` header from a stream of HTTP-style headers.
///
/// Headers are read line-by-line until an empty line (`\r\n` or `\n`) is found,
/// at which point the body is expected to follow. If the end-of-stream is hit
/// before a complete header block, `None` is returned.
///
/// Both individual header lines and the total header block are bounded to
/// prevent a client from exhausting the server's memory before the body-size
/// guard is reached.
async fn read_content_length<R: AsyncBufReadExt + Unpin>(read: &mut R) -> Option<usize> {
    let mut content_length: Option<usize> = None;
    let mut total = 0usize;

    loop {
        let line = read_header_line(read, &mut total).await?;

        // Strip the trailing newline and optional carriage return, then check
        // for the empty line that terminates the header block.
        let line = line.strip_suffix(b"\n").unwrap_or(&line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);

        if line.is_empty() {
            return content_length;
        }

        if let Some(len) = parse_content_length(line) {
            content_length = Some(len);
        }
    }
}

/// Read a single header line, stopping at the first newline and enforcing a
/// maximum line and header-block length.
async fn read_header_line<R: AsyncBufReadExt + Unpin>(
    read: &mut R,
    total: &mut usize,
) -> Option<Vec<u8>> {
    let mut line = Vec::new();

    loop {
        let remaining = MAX_HEADER_LINE_BYTES.saturating_sub(line.len());
        if remaining == 0 {
            eprintln!("csp mcp: header line exceeds {} bytes", MAX_HEADER_LINE_BYTES);
            return None;
        }

        // Scoped borrow of `read`'s internal buffer.
        let consumed = {
            let buf = match read.fill_buf().await {
                Ok(buf) if buf.is_empty() => {
                    // Stream closed. Return the partially read line only if we
                    // have something; otherwise signal EOF.
                    return if line.is_empty() { None } else { Some(line) };
                }
                Ok(buf) => buf,
                Err(_) => return None,
            };

            let search = &buf[..buf.len().min(remaining)];
            if let Some(nl) = search.iter().position(|&b| b == b'\n') {
                // Include the newline and consume it from the buffer.
                let to_read = nl + 1;
                line.extend_from_slice(&buf[..to_read]);
                to_read
            } else {
                // No newline in this chunk. Consume what we can and keep looking.
                line.extend_from_slice(search);
                search.len()
            }
        };

        read.consume(consumed);
        *total += consumed;

        if *total > MAX_HEADER_BLOCK_BYTES {
            eprintln!("csp mcp: header block exceeds {} bytes", MAX_HEADER_BLOCK_BYTES);
            return None;
        }

        // If the last chunk ended with a newline, the line is complete.
        if line.last() == Some(&b'\n') {
            return Some(line);
        }
    }
}

/// Parse a single header line of the form `Name: Value`.
///
/// Returns the value as a `usize` only when the header name is
/// `Content-Length` (case-insensitive). Malformed lines are ignored.
fn parse_content_length(line: &[u8]) -> Option<usize> {
    let mut parts = line.splitn(2, |&b| b == b':');
    let name = parts.next()?;
    let value = parts.next()?;

    if !header_name_eq(name, b"Content-Length") {
        return None;
    }

    parse_usize_bytes(trim_ascii(value))
}

/// Case-insensitive ASCII comparison of two byte slices.
fn header_name_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

/// Trim leading and trailing ASCII whitespace from a byte slice.
fn trim_ascii(mut s: &[u8]) -> &[u8] {
    while let Some(&b) = s.first() {
        if b.is_ascii_whitespace() {
            s = &s[1..];
        } else {
            break;
        }
    }
    while let Some(&b) = s.last() {
        if b.is_ascii_whitespace() {
            s = &s[..s.len() - 1];
        } else {
            break;
        }
    }
    s
}

/// Parse a non-negative decimal integer from an ASCII byte slice.
fn parse_usize_bytes(s: &[u8]) -> Option<usize> {
    if s.is_empty() {
        return None;
    }
    let mut n: usize = 0;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        n = n.checked_mul(10)?.checked_add((b - b'0') as usize)?;
    }
    Some(n)
}

/// Skip exactly `n` bytes from `read`, discarding them.
async fn skip_exact<R: AsyncReadExt + Unpin>(
    read: &mut R,
    n: usize,
) -> Result<(), std::io::Error> {
    let copied = tokio::io::copy(&mut read.take(n as u64), &mut tokio::io::sink()).await?;
    if copied as usize != n {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "unexpected EOF while skipping message body",
        ));
    }
    Ok(())
}
