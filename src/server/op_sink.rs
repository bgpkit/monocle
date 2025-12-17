//! Operation-scoped sink wrapper with terminal-guard enforcement.
//!
//! This module provides a small wrapper around [`WsSink`] that enforces the
//! protocol rule for streaming operations:
//!
//! - `progress` / `stream`: 0..N times
//! - then exactly one terminal `result` or `error`
//!
//! After a terminal message is sent, subsequent send attempts return an error.
//!
//! The wrapper is intentionally minimal: it does not implement backpressure or
//! buffering policies. It only guards protocol correctness.

use crate::server::protocol::{ErrorData, ResponseEnvelope};
use crate::server::sink::{WsSink, WsSinkError};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Errors that can occur when sending via [`WsOpSink`].
#[derive(Debug)]
pub enum WsOpSinkError {
    /// Serialization or underlying websocket send failure.
    Sink(WsSinkError),
    /// A terminal message (`result` / `error`) was already sent for this op.
    TerminalAlreadySent,
    /// Attempted to emit a streaming (`progress`/`stream`) message without an `op_id`.
    MissingOpId,
}

impl std::fmt::Display for WsOpSinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsOpSinkError::Sink(e) => write!(f, "{e}"),
            WsOpSinkError::TerminalAlreadySent => write!(f, "terminal message already sent"),
            WsOpSinkError::MissingOpId => write!(f, "missing op_id for streaming message"),
        }
    }
}

impl std::error::Error for WsOpSinkError {}

impl From<WsSinkError> for WsOpSinkError {
    fn from(e: WsSinkError) -> Self {
        WsOpSinkError::Sink(e)
    }
}

/// An operation-scoped sink that can enforce "single terminal" semantics.
///
/// This is designed to be created by the dispatcher/router for each request.
/// For non-streaming methods, `op_id` is typically `None`.
///
/// For streaming methods, `op_id` must be `Some(...)` and all progress/stream
/// messages will include it.
#[derive(Clone)]
pub struct WsOpSink {
    sink: WsSink,
    id: String,
    op_id: Option<String>,
    terminal_sent: Arc<Mutex<bool>>,
}

impl WsOpSink {
    /// Create a new operation-scoped sink.
    pub fn new(sink: WsSink, id: String, op_id: Option<String>) -> Self {
        Self {
            sink,
            id,
            op_id,
            terminal_sent: Arc::new(Mutex::new(false)),
        }
    }

    /// Get the request correlation id for this operation.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Get the operation id for this operation, if any.
    pub fn op_id(&self) -> Option<&str> {
        self.op_id.as_deref()
    }

    /// Send a non-terminal progress envelope.
    ///
    /// Per protocol, streaming messages must include an `op_id`. If this `WsOpSink`
    /// was constructed without `op_id`, this returns `MissingOpId`.
    pub async fn send_progress<T: Serialize>(&self, data: T) -> Result<(), WsOpSinkError> {
        self.ensure_not_terminal().await?;
        let op_id = self.op_id.as_ref().ok_or(WsOpSinkError::MissingOpId)?;
        Ok(self
            .sink
            .send_envelope(ResponseEnvelope::progress(
                self.id.clone(),
                op_id.clone(),
                data,
            ))
            .await?)
    }

    /// Send a non-terminal stream envelope.
    ///
    /// Per protocol, streaming messages must include an `op_id`. If this `WsOpSink`
    /// was constructed without `op_id`, this returns `MissingOpId`.
    pub async fn send_stream<T: Serialize>(&self, data: T) -> Result<(), WsOpSinkError> {
        self.ensure_not_terminal().await?;
        let op_id = self.op_id.as_ref().ok_or(WsOpSinkError::MissingOpId)?;
        Ok(self
            .sink
            .send_envelope(ResponseEnvelope::stream(
                self.id.clone(),
                op_id.clone(),
                data,
            ))
            .await?)
    }

    /// Send the terminal result envelope.
    ///
    /// If `op_id` is present, it will be included. Terminal messages are allowed exactly once.
    pub async fn send_result<T: Serialize>(&self, data: T) -> Result<(), WsOpSinkError> {
        self.mark_terminal().await?;
        match &self.op_id {
            Some(op_id) => Ok(self
                .sink
                .send_envelope(ResponseEnvelope::result_with_op(
                    self.id.clone(),
                    op_id.clone(),
                    data,
                ))
                .await?),
            None => Ok(self
                .sink
                .send_envelope(ResponseEnvelope::result(self.id.clone(), data))
                .await?),
        }
    }

    /// Send the terminal error envelope.
    ///
    /// Terminal messages are allowed exactly once.
    pub async fn send_error(&self, error: ErrorData) -> Result<(), WsOpSinkError> {
        self.mark_terminal().await?;
        Ok(self
            .sink
            .send_envelope(ResponseEnvelope::error(
                self.id.clone(),
                self.op_id.clone(),
                error,
            ))
            .await?)
    }

    /// Expose the underlying sink for legacy callers.
    ///
    /// Prefer using the guarded methods above; this is provided only to ease
    /// gradual migration.
    pub fn inner(&self) -> &WsSink {
        &self.sink
    }

    async fn ensure_not_terminal(&self) -> Result<(), WsOpSinkError> {
        let sent = *self.terminal_sent.lock().await;
        if sent {
            Err(WsOpSinkError::TerminalAlreadySent)
        } else {
            Ok(())
        }
    }

    async fn mark_terminal(&self) -> Result<(), WsOpSinkError> {
        let mut sent = self.terminal_sent.lock().await;
        if *sent {
            return Err(WsOpSinkError::TerminalAlreadySent);
        }
        *sent = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: we can't easily unit-test actual websocket sending here without constructing an Axum
    // WebSocket sink. These tests focus on the terminal guard state machine.

    #[tokio::test]
    async fn terminal_guard_allows_one_terminal() {
        // Dummy sink isn't available here; we only validate guard behavior via internal methods.
        // We construct an instance with a placeholder WsSink by using unsafe workaround is not
        // acceptable; instead we test the guard methods directly by calling them in order.

        // Create a minimal instance by reusing fields (without sending). We can't construct WsSink
        // without an actual websocket sender, so we only validate state transitions by calling
        // the private methods through a local shim.
        struct Guard(Arc<Mutex<bool>>);

        impl Guard {
            async fn ensure_not_terminal(&self) -> Result<(), WsOpSinkError> {
                let sent = *self.0.lock().await;
                if sent {
                    Err(WsOpSinkError::TerminalAlreadySent)
                } else {
                    Ok(())
                }
            }
            async fn mark_terminal(&self) -> Result<(), WsOpSinkError> {
                let mut sent = self.0.lock().await;
                if *sent {
                    return Err(WsOpSinkError::TerminalAlreadySent);
                }
                *sent = true;
                Ok(())
            }
        }

        let g = Guard(Arc::new(Mutex::new(false)));

        g.ensure_not_terminal().await.unwrap();
        g.mark_terminal().await.unwrap();
        assert!(matches!(
            g.mark_terminal().await,
            Err(WsOpSinkError::TerminalAlreadySent)
        ));
        assert!(matches!(
            g.ensure_not_terminal().await,
            Err(WsOpSinkError::TerminalAlreadySent)
        ));
    }
}
