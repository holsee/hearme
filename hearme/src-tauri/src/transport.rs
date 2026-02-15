//! iroh-based P2P transport for audio streaming.
//!
//! Two modes:
//! - **Share**: captures app audio, encodes Opus, serves to connecting listeners
//! - **Listen**: connects to a sharer, receives Opus packets, decodes to PCM
//!
//! Wire protocol (per Opus frame on the QUIC stream):
//!   [u16 LE length][opus packet bytes]
//!
//! 1-to-many: each listener opens its own bi-stream. The sharer spawns a task
//! per listener that reads from a broadcast channel of encoded frames.

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, SecretKey};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn};

/// Custom ALPN for hearme audio streams.
const ALPN: &[u8] = b"/hearme/audio/1";

/// A ticket that a listener uses to connect to a sharer.
/// Serialized as JSON then base64-encoded for easy copy/paste.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    pub addr: EndpointAddr,
}

impl Ticket {
    /// Encode ticket to a copy-pasteable string.
    pub fn to_string_encoded(&self) -> Result<String> {
        let json = serde_json::to_vec(self)?;
        Ok(data_encoding::BASE64URL_NOPAD.encode(&json))
    }

    /// Decode ticket from the encoded string.
    pub fn from_string_encoded(s: &str) -> Result<Self> {
        let json = data_encoding::BASE64URL_NOPAD.decode(s.trim().as_bytes())?;
        Ok(serde_json::from_slice(&json)?)
    }
}

// ─── Sharer (server) side ───────────────────────────────────────────

/// Handle to an active sharing session. Drop to stop.
pub struct ShareSession {
    router: Router,
    /// Send encoded Opus frames here; all connected listeners receive them.
    pub opus_tx: broadcast::Sender<Arc<Vec<u8>>>,
}

impl ShareSession {
    /// Start sharing. Returns the session and a ticket for listeners.
    pub async fn start() -> Result<(Self, Ticket)> {
        let endpoint = Endpoint::builder()
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await?;

        endpoint.online().await;
        let addr = endpoint.addr();
        let ticket = Ticket { addr };

        info!("Sharing on endpoint: {}", endpoint.id());

        // Broadcast channel: sharer writes encoded frames, listeners read.
        // Buffer 50 frames (~1 second of audio) before dropping oldest.
        let (opus_tx, _) = broadcast::channel::<Arc<Vec<u8>>>(50);

        let handler = AudioShareHandler {
            opus_tx: opus_tx.clone(),
        };

        let router = Router::builder(endpoint).accept(ALPN, handler).spawn();

        Ok((Self { router, opus_tx }, ticket))
    }

    /// Shut down the sharing session.
    pub async fn stop(self) -> Result<()> {
        self.router.shutdown().await?;
        Ok(())
    }
}

/// Protocol handler: accepts connections from listeners and streams audio.
#[derive(Debug, Clone)]
struct AudioShareHandler {
    opus_tx: broadcast::Sender<Arc<Vec<u8>>>,
}

impl ProtocolHandler for AudioShareHandler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let mut opus_rx = self.opus_tx.subscribe();
        let remote = connection.remote_id();
        info!("Listener connected: {remote}");

        // Accept a bi-stream from the listener (they open it to signal readiness)
        let (mut send, _recv) = connection.accept_bi().await?;

        // Stream Opus frames to this listener
        loop {
            match opus_rx.recv().await {
                Ok(packet) => {
                    let len = packet.len() as u16;
                    // Write length-prefixed packet
                    if send.write_all(&len.to_le_bytes()).await.is_err() {
                        break;
                    }
                    if send.write_all(&packet).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    warn!("Listener {remote} lagged by {n} frames, skipping");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }

        info!("Listener disconnected: {remote}");
        Ok(())
    }
}

// ─── Listener (client) side ─────────────────────────────────────────

/// Handle to a listening session. Drop to stop.
pub struct ListenSession {
    endpoint: Endpoint,
    stop_tx: tokio::sync::oneshot::Sender<()>,
}

impl ListenSession {
    /// Connect to a sharer and start receiving audio.
    /// Returns decoded PCM frames via the mpsc channel.
    pub async fn connect(ticket: &Ticket) -> Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        let endpoint = Endpoint::bind().await?;
        endpoint.online().await;

        let conn = endpoint
            .connect(ticket.addr.clone(), ALPN)
            .await
            .context("Failed to connect to sharer")?;

        info!("Connected to sharer: {}", conn.remote_id());

        // Open bi-stream to signal we're ready
        let (send, mut recv) = conn.open_bi().await.context("Failed to open bi-stream")?;

        let (opus_tx, opus_rx) = mpsc::channel::<Vec<u8>>(64);
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();

        // Spawn receive loop
        tokio::spawn(async move {
            let mut len_buf = [0u8; 2];
            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    result = recv.read_exact(&mut len_buf) => {
                        match result {
                            Ok(()) => {},
                            Err(_) => break,
                        }
                        let len = u16::from_le_bytes(len_buf) as usize;
                        let mut packet = vec![0u8; len];
                        match recv.read_exact(&mut packet).await {
                            Ok(()) => {
                                if opus_tx.send(packet).await.is_err() {
                                    break; // receiver dropped
                                }
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
            drop(send); // close our end
            info!("Listen session ended");
        });

        Ok((Self { endpoint, stop_tx }, opus_rx))
    }

    /// Disconnect from the sharer.
    pub async fn stop(self) {
        let _ = self.stop_tx.send(());
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ticket_round_trip() {
        // Create an endpoint just to get a real EndpointAddr
        let endpoint = Endpoint::builder().bind().await.unwrap();
        endpoint.online().await;
        let addr = endpoint.addr();

        let ticket = Ticket { addr: addr.clone() };

        // Encode to string
        let encoded = ticket.to_string_encoded().unwrap();
        assert!(!encoded.is_empty());

        // Should be valid base64url (no padding, no +/)
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));

        // Decode back
        let decoded = Ticket::from_string_encoded(&encoded).unwrap();

        // The node ID should match
        assert_eq!(format!("{:?}", ticket.addr), format!("{:?}", decoded.addr),);

        endpoint.close().await;
    }

    #[tokio::test]
    async fn ticket_round_trip_with_whitespace() {
        let endpoint = Endpoint::builder().bind().await.unwrap();
        endpoint.online().await;

        let ticket = Ticket {
            addr: endpoint.addr(),
        };
        let encoded = ticket.to_string_encoded().unwrap();

        // Should tolerate leading/trailing whitespace (e.g. from copy-paste)
        let padded = format!("  {encoded}\n");
        let decoded = Ticket::from_string_encoded(&padded);
        assert!(decoded.is_ok());

        endpoint.close().await;
    }

    #[test]
    fn ticket_from_invalid_base64_fails() {
        let result = Ticket::from_string_encoded("not!valid!base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn ticket_from_valid_base64_invalid_json_fails() {
        // Valid base64url but not valid JSON
        let encoded = data_encoding::BASE64URL_NOPAD.encode(b"not json");
        let result = Ticket::from_string_encoded(&encoded);
        assert!(result.is_err());
    }

    #[test]
    fn alpn_is_correct() {
        assert_eq!(ALPN, b"/hearme/audio/1");
    }
}
