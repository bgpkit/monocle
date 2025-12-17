//! WebSocket sink abstraction (transport primitive)
//!
//! `WsSink` is intentionally minimal: it is a thin wrapper around the Axum WebSocket
//! sender and provides only transport-level primitives.
//!
//! Higher-level protocol semantics (result/progress/stream/error) are handled by
//! `WsOpSink` and `ResponseEnvelope` helpers.

use crate::server::protocol::ResponseEnvelope;
use axum::extract::ws::{Message, WebSocket};
use futures::stream::SplitSink;
use futures::SinkExt;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Minimal wrapper around the WebSocket sender.
///
/// This type should not grow protocol-specific helpers. Keep it transport-only.
#[derive(Clone)]
pub struct WsSink {
    pub(crate) inner: Arc<Mutex<SplitSink<WebSocket, Message>>>,
}

impl WsSink {
    /// Create a new `WsSink` from a WebSocket sender.
    pub fn new(sender: SplitSink<WebSocket, Message>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(sender)),
        }
    }

    /// Send a raw websocket message (server internal use: pong/close/etc).
    pub async fn send_message_raw(&self, msg: Message) -> Result<(), WsSinkError> {
        let mut sender = self.inner.lock().await;
        sender
            .send(msg)
            .await
            .map_err(|e| WsSinkError::SendError(e.to_string()))
    }

    /// Send a protocol response envelope as a JSON text websocket message.
    pub async fn send_envelope(&self, envelope: ResponseEnvelope) -> Result<(), WsSinkError> {
        let json = serde_json::to_string(&envelope)
            .map_err(|e| WsSinkError::SerializationError(e.to_string()))?;
        self.send_message_raw(Message::Text(json)).await
    }
}

/// Errors that can occur when sending messages
#[derive(Debug, Clone)]
pub enum WsSinkError {
    /// Failed to serialize message
    SerializationError(String),
    /// Failed to send message
    SendError(String),
}

impl std::fmt::Display for WsSinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsSinkError::SerializationError(e) => write!(f, "Serialization error: {}", e),
            WsSinkError::SendError(e) => write!(f, "Send error: {}", e),
        }
    }
}

impl std::error::Error for WsSinkError {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_sink_error_display() {
        let err = WsSinkError::SerializationError("test".to_string());
        assert!(err.to_string().contains("Serialization error"));

        let err = WsSinkError::SendError("connection closed".to_string());
        assert!(err.to_string().contains("Send error"));
    }
}
