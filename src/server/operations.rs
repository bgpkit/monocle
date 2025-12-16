//! Operation registry for managing streaming operations and cancellation
//!
//! This module provides the `OperationRegistry` which tracks active streaming
//! operations and enables cancellation via op_id.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

// =============================================================================
// Operation Status
// =============================================================================

/// Status of an operation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    /// Operation is currently running
    Running,
    /// Operation completed successfully
    Completed,
    /// Operation failed with an error
    Failed,
    /// Operation was cancelled
    Cancelled,
}

// =============================================================================
// Operation Entry
// =============================================================================

/// Information about a tracked operation
#[derive(Debug)]
pub struct OperationEntry {
    /// The request ID associated with this operation
    pub request_id: String,

    /// The method name
    pub method: String,

    /// Current status
    pub status: OperationStatus,

    /// Cancellation token for this operation
    pub cancel_token: CancellationToken,

    /// When the operation was started
    pub started_at: std::time::Instant,
}

impl OperationEntry {
    /// Create a new operation entry
    pub fn new(request_id: String, method: String) -> Self {
        Self {
            request_id,
            method,
            status: OperationStatus::Running,
            cancel_token: CancellationToken::new(),
            started_at: std::time::Instant::now(),
        }
    }

    /// Check if the operation is still running
    pub fn is_running(&self) -> bool {
        self.status == OperationStatus::Running
    }

    /// Check if the operation was cancelled
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }

    /// Get the elapsed time since the operation started
    pub fn elapsed(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
}

// =============================================================================
// Operation Registry
// =============================================================================

/// Registry for tracking active operations
///
/// This registry allows:
/// - Registering new streaming operations with generated op_ids
/// - Looking up operations by op_id
/// - Cancelling operations by op_id
/// - Tracking operation status
/// - Enforcing concurrency limits
#[derive(Default)]
pub struct OperationRegistry {
    /// Map from op_id to operation entry
    operations: RwLock<HashMap<String, Arc<Mutex<OperationEntry>>>>,

    /// Maximum concurrent operations (0 = unlimited)
    max_concurrent: usize,

    /// Number of currently-running operations (O(1) concurrency check).
    running: AtomicUsize,
}

impl OperationRegistry {
    /// Create a new operation registry
    pub fn new() -> Self {
        Self {
            operations: RwLock::new(HashMap::new()),
            max_concurrent: 0,
            running: AtomicUsize::new(0),
        }
    }

    /// Create a new operation registry with a concurrency limit
    pub fn with_max_concurrent(max: usize) -> Self {
        Self {
            operations: RwLock::new(HashMap::new()),
            max_concurrent: max,
            running: AtomicUsize::new(0),
        }
    }

    /// Generate a new operation ID
    fn generate_op_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Register a new operation
    ///
    /// Returns the generated op_id and cancellation token, or an error if
    /// the concurrency limit has been reached.
    pub async fn register(
        &self,
        request_id: String,
        method: String,
    ) -> Result<(String, CancellationToken), RegistryError> {
        // Fast-path concurrency check without scanning/locking the map.
        if self.max_concurrent > 0 {
            let current = self.running.load(Ordering::Relaxed);
            if current >= self.max_concurrent {
                return Err(RegistryError::ConcurrencyLimitReached);
            }
        }

        // Register into the map and increment running count for this newly-running op.
        let mut ops = self.operations.write().await;

        // Re-check under the write lock to avoid oversubscription when max_concurrent is set.
        if self.max_concurrent > 0 {
            let current = self.running.load(Ordering::Relaxed);
            if current >= self.max_concurrent {
                return Err(RegistryError::ConcurrencyLimitReached);
            }
        }

        let op_id = Self::generate_op_id();
        let entry = OperationEntry::new(request_id, method);
        let cancel_token = entry.cancel_token.clone();

        ops.insert(op_id.clone(), Arc::new(Mutex::new(entry)));
        self.running.fetch_add(1, Ordering::Relaxed);

        Ok((op_id, cancel_token))
    }

    /// Get an operation by op_id
    pub async fn get(&self, op_id: &str) -> Option<Arc<Mutex<OperationEntry>>> {
        let ops = self.operations.read().await;
        ops.get(op_id).cloned()
    }

    /// Cancel an operation by op_id
    ///
    /// Returns true if the operation was found and cancelled, false if not found.
    pub async fn cancel(&self, op_id: &str) -> Result<(), RegistryError> {
        let ops = self.operations.read().await;

        match ops.get(op_id) {
            Some(entry) => {
                let mut entry = entry.lock().await;
                if entry.status == OperationStatus::Running {
                    entry.status = OperationStatus::Cancelled;
                    entry.cancel_token.cancel();
                    self.running.fetch_sub(1, Ordering::Relaxed);
                    Ok(())
                } else {
                    Err(RegistryError::OperationNotRunning)
                }
            }
            None => Err(RegistryError::OperationNotFound),
        }
    }

    /// Mark an operation as completed
    pub async fn complete(&self, op_id: &str) -> Result<(), RegistryError> {
        let ops = self.operations.read().await;

        match ops.get(op_id) {
            Some(entry) => {
                let mut entry = entry.lock().await;
                if entry.status == OperationStatus::Running {
                    entry.status = OperationStatus::Completed;
                    self.running.fetch_sub(1, Ordering::Relaxed);
                }
                Ok(())
            }
            None => Err(RegistryError::OperationNotFound),
        }
    }

    /// Mark an operation as failed
    pub async fn fail(&self, op_id: &str) -> Result<(), RegistryError> {
        let ops = self.operations.read().await;

        match ops.get(op_id) {
            Some(entry) => {
                let mut entry = entry.lock().await;
                if entry.status == OperationStatus::Running {
                    entry.status = OperationStatus::Failed;
                    self.running.fetch_sub(1, Ordering::Relaxed);
                }
                Ok(())
            }
            None => Err(RegistryError::OperationNotFound),
        }
    }

    /// Remove an operation from the registry
    ///
    /// Note: this does not change the running counter. Use `complete`/`fail`/`cancel`
    /// to transition out of Running, then `remove` (or `complete_and_remove`, etc).
    pub async fn remove(&self, op_id: &str) -> Option<Arc<Mutex<OperationEntry>>> {
        let mut ops = self.operations.write().await;
        ops.remove(op_id)
    }

    /// Mark an operation as completed and remove it from the registry.
    pub async fn complete_and_remove(&self, op_id: &str) -> Result<(), RegistryError> {
        self.complete(op_id).await?;
        self.remove(op_id).await;
        Ok(())
    }

    /// Mark an operation as failed and remove it from the registry.
    pub async fn fail_and_remove(&self, op_id: &str) -> Result<(), RegistryError> {
        self.fail(op_id).await?;
        self.remove(op_id).await;
        Ok(())
    }

    /// Mark an operation as cancelled and remove it from the registry.
    pub async fn cancel_and_remove(&self, op_id: &str) -> Result<(), RegistryError> {
        self.cancel(op_id).await?;
        self.remove(op_id).await;
        Ok(())
    }

    /// Get the count of running operations
    pub async fn running_count(&self) -> usize {
        self.running.load(Ordering::Relaxed)
    }

    /// Get all operation IDs
    pub async fn op_ids(&self) -> Vec<String> {
        let ops = self.operations.read().await;
        ops.keys().cloned().collect()
    }

    /// Clean up completed/failed/cancelled operations older than the given duration
    pub async fn cleanup(&self, older_than: std::time::Duration) {
        let mut ops = self.operations.write().await;
        let now = std::time::Instant::now();

        ops.retain(|_, entry| {
            if let Ok(e) = entry.try_lock() {
                // Keep running operations and recent non-running operations
                e.is_running() || now.duration_since(e.started_at) < older_than
            } else {
                true // Keep if we can't check
            }
        });
    }
}

// =============================================================================
// Errors
// =============================================================================

/// Errors that can occur with the operation registry
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// Operation not found
    OperationNotFound,
    /// Operation is not running
    OperationNotRunning,
    /// Concurrency limit reached
    ConcurrencyLimitReached,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::OperationNotFound => write!(f, "Operation not found"),
            RegistryError::OperationNotRunning => write!(f, "Operation is not running"),
            RegistryError::ConcurrencyLimitReached => write!(f, "Concurrency limit reached"),
        }
    }
}

impl std::error::Error for RegistryError {}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_operation() {
        let registry = OperationRegistry::new();

        let (op_id, _cancel_token) = registry
            .register("req-1".to_string(), "parse.start".to_string())
            .await
            .unwrap();

        assert!(!op_id.is_empty());
        assert_eq!(registry.running_count().await, 1);
    }

    #[tokio::test]
    async fn test_cancel_operation() {
        let registry = OperationRegistry::new();

        let (op_id, cancel_token) = registry
            .register("req-1".to_string(), "parse.start".to_string())
            .await
            .unwrap();

        assert!(!cancel_token.is_cancelled());

        registry.cancel(&op_id).await.unwrap();

        assert!(cancel_token.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancel_unknown_operation() {
        let registry = OperationRegistry::new();

        let result = registry.cancel("unknown-op-id").await;
        assert_eq!(result, Err(RegistryError::OperationNotFound));
    }

    #[tokio::test]
    async fn test_concurrency_limit() {
        let registry = OperationRegistry::with_max_concurrent(2);

        // Register first two operations
        let (op_id1, _) = registry
            .register("req-1".to_string(), "parse.start".to_string())
            .await
            .unwrap();
        let (_op_id2, _) = registry
            .register("req-2".to_string(), "parse.start".to_string())
            .await
            .unwrap();

        // Third should fail
        let result = registry
            .register("req-3".to_string(), "parse.start".to_string())
            .await;

        // `register()` returns `Result<(String, CancellationToken), RegistryError>`, but
        // `CancellationToken` doesn't implement `PartialEq`, so we can't `assert_eq!` on the
        // full `Result`. Instead, assert the error variant.
        assert!(matches!(
            result,
            Err(RegistryError::ConcurrencyLimitReached)
        ));

        // Complete one operation
        registry.complete(&op_id1).await.unwrap();

        // Now we should be able to register another
        let result = registry
            .register("req-3".to_string(), "parse.start".to_string())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_complete_operation() {
        let registry = OperationRegistry::new();

        let (op_id, _) = registry
            .register("req-1".to_string(), "parse.start".to_string())
            .await
            .unwrap();

        assert_eq!(registry.running_count().await, 1);

        registry.complete(&op_id).await.unwrap();

        assert_eq!(registry.running_count().await, 0);
    }

    #[tokio::test]
    async fn test_fail_operation() {
        let registry = OperationRegistry::new();

        let (op_id, _) = registry
            .register("req-1".to_string(), "parse.start".to_string())
            .await
            .unwrap();

        registry.fail(&op_id).await.unwrap();

        let entry = registry.get(&op_id).await.unwrap();
        let entry = entry.lock().await;
        assert_eq!(entry.status, OperationStatus::Failed);
    }

    #[tokio::test]
    async fn test_remove_operation() {
        let registry = OperationRegistry::new();

        let (op_id, _) = registry
            .register("req-1".to_string(), "parse.start".to_string())
            .await
            .unwrap();

        assert!(registry.get(&op_id).await.is_some());

        registry.remove(&op_id).await;

        assert!(registry.get(&op_id).await.is_none());
    }
}
