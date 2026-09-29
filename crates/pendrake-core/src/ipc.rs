//! IPC server. Newline-delimited JSON, one task per client, over the platform
//! transport (Unix socket or Windows named pipe, see [`crate::transport`]).
//!
//! A connection is request/response until it sends `subscribeEvents`; from then
//! on the daemon also pushes [`SyncEvent`] lines as the wallet scans, interleaved
//! with any further replies on the same connection.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use pendrake_ipc::{Request, Response, SyncEvent};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf,
    WriteHalf,
};
use tokio::sync::broadcast::error::RecvError;
use zeroize::Zeroizing;

use crate::paths::Paths;
use crate::transport::Listener;
use crate::wallet_service::WalletService;

/// The longest request line accepted. An import carries a UFVK of a few hundred
/// bytes, so this is generous; it exists so a peer that never sends a newline
/// cannot grow the read buffer until the daemon dies.
const MAX_LINE_BYTES: usize = 64 * 1024;

pub async fn serve(service: Arc<WalletService>, paths: Paths) -> Result<()> {
    let endpoint = paths.endpoint();
    let mut listener =
        Listener::bind(&endpoint).with_context(|| format!("binding endpoint {endpoint}"))?;
    tracing::info!("listening on {endpoint}");

    loop {
        let conn = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("accept failed: {e}");
                continue;
            }
        };
        let service = Arc::clone(&service);
        tokio::spawn(async move {
            if let Err(e) = handle_conn(conn, service).await {
                tracing::debug!("connection closed: {e}");
            }
        });
    }
}

async fn handle_conn<S>(stream: S, service: Arc<WalletService>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut buf = Vec::new();

    while let Some(line) = read_frame(&mut reader, &mut buf).await? {
        let line = Zeroizing::new(line);
        if line.trim().is_empty() {
            continue;
        }
        let (resp, subscribe) = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                let subscribe = req.method == "subscribeEvents";
                let resp = match service.handle(&req.method, req.params).await {
                    Ok(result) => Response::ok(req.id, result),
                    Err(e) => Response::err(req.id, e.to_string()),
                };
                (resp, subscribe)
            }
            Err(e) => (Response::err(0, format!("bad request: {e}")), false),
        };
        write_line(&mut write_half, &resp).await?;
        if subscribe {
            return stream_events(reader, write_half, service).await;
        }
    }
    Ok(())
}

/// Read one newline-delimited frame. A bare tail without a newline still counts.
/// A line past [`MAX_LINE_BYTES`] is a protocol error.
async fn read_frame<R>(reader: &mut BufReader<R>, buf: &mut Vec<u8>) -> Result<Option<String>>
where
    R: AsyncRead + Unpin,
{
    buf.clear();
    loop {
        let remaining = (MAX_LINE_BYTES + 1).saturating_sub(buf.len()) as u64;
        let n = reader.take(remaining).read_until(b'\n', buf).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            let line = std::str::from_utf8(buf)?.to_string();
            buf.clear();
            return Ok(Some(line));
        }
        if buf.last() == Some(&b'\n') {
            buf.pop();
            if buf.last() == Some(&b'\r') {
                buf.pop();
            }
            let line = std::str::from_utf8(buf)?.to_string();
            buf.clear();
            return Ok(Some(line));
        }
        if buf.len() > MAX_LINE_BYTES {
            bail!("request line exceeds {MAX_LINE_BYTES} bytes");
        }
    }
}

async fn stream_events<S>(
    mut reader: BufReader<ReadHalf<S>>,
    mut write_half: WriteHalf<S>,
    service: Arc<WalletService>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite,
{
    let mut buf = Vec::new();
    service.subscriber_joined();
    let _subscriber = SubscriberGuard(Arc::clone(&service));

    let mut events = service.subscribe();
    loop {
        tokio::select! {
            frame = read_frame(&mut reader, &mut buf) => {
                let Some(line) = frame? else { break };
                let line = Zeroizing::new(line);
                if line.trim().is_empty() {
                    continue;
                }
                let resp = match serde_json::from_str::<Request>(&line) {
                    Ok(req) => match service.handle(&req.method, req.params).await {
                        Ok(result) => Response::ok(req.id, result),
                        Err(e) => Response::err(req.id, e.to_string()),
                    },
                    Err(e) => Response::err(0, format!("bad request: {e}")),
                };
                write_line(&mut write_half, &resp).await?;
            }

            event = events.recv() => match event {
                Ok(event) => {
                    if !service.session_locked() || matches!(event, SyncEvent::Error { .. }) {
                        write_line(&mut write_half, &event).await?;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            },
        }
    }
    Ok(())
}

struct SubscriberGuard(Arc<WalletService>);

impl Drop for SubscriberGuard {
    fn drop(&mut self) {
        self.0.subscriber_left();
    }
}

async fn write_line<W, T>(write_half: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: serde::Serialize,
{
    let mut encoded = serde_json::to_vec(value)?;
    encoded.push(b'\n');
    write_half.write_all(&encoded).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncWriteExt, BufReader};

    use super::{read_frame, MAX_LINE_BYTES};

    #[tokio::test]
    async fn frames_split_on_newlines_and_a_bare_tail_still_counts() {
        let (mut client, server) = tokio::io::duplex(1024);
        let mut reader = BufReader::new(server);
        let mut buf = Vec::new();

        client.write_all(b"one\ntwo\ntail").await.unwrap();
        drop(client);

        assert_eq!(
            read_frame(&mut reader, &mut buf).await.unwrap().as_deref(),
            Some("one")
        );
        assert_eq!(
            read_frame(&mut reader, &mut buf).await.unwrap().as_deref(),
            Some("two")
        );
        assert_eq!(
            read_frame(&mut reader, &mut buf).await.unwrap().as_deref(),
            Some("tail")
        );
        assert!(read_frame(&mut reader, &mut buf).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_line_past_the_cap_is_refused_before_a_newline_arrives() {
        let (mut client, server) = tokio::io::duplex(4096);
        let mut reader = BufReader::new(server);
        let mut buf = Vec::new();

        let writer = tokio::spawn(async move {
            let chunk = vec![b'x'; 4096];
            for _ in 0..(MAX_LINE_BYTES / 4096 + 2) {
                if client.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        });

        let err = read_frame(&mut reader, &mut buf).await.unwrap_err();
        assert!(err.to_string().contains("exceeds"));
        drop(reader);
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn a_line_at_the_cap_is_still_accepted() {
        let (mut client, server) = tokio::io::duplex(4096);
        let mut reader = BufReader::new(server);
        let mut buf = Vec::new();

        let writer = tokio::spawn(async move {
            let mut line = vec![b'y'; MAX_LINE_BYTES];
            line.push(b'\n');
            client.write_all(&line).await.unwrap();
        });

        let line = read_frame(&mut reader, &mut buf).await.unwrap().unwrap();
        assert_eq!(line.len(), MAX_LINE_BYTES);
        writer.await.unwrap();
    }
}
