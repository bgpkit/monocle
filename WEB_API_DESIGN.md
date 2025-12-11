# Monocle Web API Design

This document describes the design for automatically generating RESTful and WebSocket APIs for monocle's lens modules.

## Overview

The goal is to ensure that **every lens automatically has corresponding web endpoints** through a combination of:
1. **Traits** that define the web interface contract
2. **Macros** that reduce boilerplate for endpoint handlers
3. **Feature flags** to conditionally compile server support

## Design Principles

1. **Automatic Endpoint Generation**: Every lens should automatically expose web endpoints without manual handler code
2. **Type Safety**: Request/response types are derived from existing Args/Output types
3. **Consistency**: Uniform API patterns across all lenses
4. **Streaming Support**: WebSocket for long-running operations (parse, search)
5. **Documentation**: Auto-generated OpenAPI specs from types
6. **Resource Management**: Proper cleanup on client disconnection
7. **UI-First Design**: API designed with web and desktop UI consumers in mind

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              Web Layer                                      │
│                                                                             │
│  ┌─────────────────────┐     ┌─────────────────────┐                       │
│  │    REST Handler     │     │  WebSocket Handler  │                       │
│  │  (Request/Response) │     │    (Streaming)      │                       │
│  └──────────┬──────────┘     └──────────┬──────────┘                       │
│             │                           │                                   │
│             └─────────────┬─────────────┘                                   │
│                           ▼                                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐   │
│  │                         Router / Dispatcher                          │   │
│  │   Auto-generated from lens registrations                            │   │
│  └─────────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Trait Layer                                      │
│                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │  WebLens trait                                                        │  │
│  │  - lens_name() -> &'static str                                       │  │
│  │  - operations() -> Vec<Operation>                                    │  │
│  │  - handle_request(op, payload) -> Result<Response>                   │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
│                                                                             │
│  ┌─────────────────────────┐     ┌─────────────────────────────────────┐   │
│  │   RestOperation trait   │     │     StreamOperation trait           │   │
│  │   - Single request      │     │     - Streaming responses           │   │
│  │   - JSON response       │     │     - WebSocket messages            │   │
│  └─────────────────────────┘     └─────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Lens Layer                                       │
│                                                                             │
│  ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐    │
│  │ TimeLens  │ │  IpLens   │ │ RpkiLens  │ │As2orgLens │ │SearchLens │    │
│  └───────────┘ └───────────┘ └───────────┘ └───────────┘ └───────────┘    │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Traits

### Base Lens Trait

All lenses implement a common `Lens` trait that provides introspection and ensures consistency:

```rust
// src/lens/traits.rs

use serde::{Deserialize, Serialize};

/// Category of lens based on its data requirements
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LensCategory {
    /// Standalone lens with no external dependencies
    Standalone,
    /// Requires database connection
    DatabaseBacked,
    /// Requires external API calls
    ExternalApi,
    /// Processes files (potentially large/streaming)
    FileProcessing,
}

/// Base trait for all lenses
/// 
/// This trait provides common functionality and introspection for all lens types.
/// When the `server` feature is enabled, lenses must also implement `WebLens`.
pub trait Lens: Send + Sync {
    /// Unique identifier for this lens
    fn name(&self) -> &'static str;
    
    /// Human-readable description
    fn description(&self) -> &'static str;
    
    /// Category of this lens
    fn category(&self) -> LensCategory;
    
    /// Version of the lens implementation
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    
    /// Whether this lens requires initialization before use
    fn requires_init(&self) -> bool {
        false
    }
    
    /// Initialize the lens (e.g., load data, bootstrap database)
    /// Called automatically by the server on startup
    fn init(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

/// Compile-time enforcement: when server feature is enabled,
/// all Lens implementations must also implement WebLens
#[cfg(feature = "server")]
pub trait LensExt: Lens + WebLens {}

#[cfg(feature = "server")]
impl<T: Lens + WebLens> LensExt for T {}
```

### Web Traits

```rust
// src/server/traits.rs

use serde::{Deserialize, Serialize};
use std::pin::Pin;
use futures::Stream;
use tokio_util::sync::CancellationToken;

/// Metadata about a web operation
#[derive(Debug, Clone, Serialize)]
pub struct OperationMeta {
    pub name: &'static str,
    pub method: HttpMethod,
    pub path: &'static str,
    pub description: &'static str,
    pub streaming: bool,
    /// Whether this operation supports pagination
    pub paginated: bool,
    /// Estimated resource intensity (for queue prioritization)
    pub resource_intensity: ResourceIntensity,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub enum HttpMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ResourceIntensity {
    /// Fast, in-memory operations (Time, Country lookup)
    Low,
    /// Moderate operations (database queries, small API calls)
    Medium,
    /// Heavy operations (file parsing, multi-file search)
    High,
}

/// Base trait for all web-enabled lenses
pub trait WebLens: Send + Sync {
    /// Unique identifier for this lens (used in URL paths)
    fn lens_name(&self) -> &'static str;
    
    /// List all available operations
    fn operations(&self) -> Vec<OperationMeta>;
    
    /// Handle a REST request
    fn handle_rest(
        &self,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, WebError>;
    
    /// Check if this lens supports streaming for an operation
    fn supports_streaming(&self, operation: &str) -> bool;
    
    /// Check if this lens requires data initialization
    fn needs_data_init(&self) -> bool {
        false
    }
    
    /// Initialize/bootstrap data for this lens
    /// Called automatically on server startup for database-backed lenses
    fn init_data(&self) -> Result<(), WebError> {
        Ok(())
    }
}

/// Trait for streaming operations (WebSocket)
pub trait StreamLens: WebLens {
    /// Handle a streaming request, returning a stream of results
    /// 
    /// The `cancel_token` should be checked periodically and the stream
    /// should terminate when cancelled (e.g., client disconnection).
    fn handle_stream(
        &self,
        operation: &str,
        payload: serde_json::Value,
        cancel_token: CancellationToken,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<serde_json::Value, WebError>> + Send>>, WebError>;
}

/// Marker trait for request types
pub trait WebRequest: Serialize + for<'de> Deserialize<'de> + Send {
    /// Validate the request parameters
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Marker trait for response types
pub trait WebResponse: Serialize + Send {}

/// Web error type
#[derive(Debug, Clone, Serialize)]
pub struct WebError {
    pub code: u16,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl WebError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self { code: 400, message: message.into(), details: None }
    }
    
    pub fn not_found(message: impl Into<String>) -> Self {
        Self { code: 404, message: message.into(), details: None }
    }
    
    pub fn internal(message: impl Into<String>) -> Self {
        Self { code: 500, message: message.into(), details: None }
    }
    
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self { code: 503, message: message.into(), details: None }
    }
    
    pub fn too_many_requests(queue_position: usize) -> Self {
        Self { 
            code: 429, 
            message: "Too many requests".into(), 
            details: Some(serde_json::json!({ "queue_position": queue_position })),
        }
    }
}
```

### Derive Macro

The `#[derive(WebLens)]` macro auto-generates the `WebLens` implementation:

```rust
// In monocle-macros crate (or using inventory/linkme for registration)

/// Derive macro for WebLens trait
/// 
/// Usage:
/// ```rust
/// #[derive(WebLens)]
/// #[web_lens(name = "time")]
/// pub struct TimeLens;
/// 
/// #[web_operation(method = "get", path = "/parse")]
/// impl TimeLens {
///     pub fn parse(&self, args: &TimeParseArgs) -> Result<Vec<TimeBgpTime>> { ... }
/// }
/// ```
#[proc_macro_derive(WebLens, attributes(web_lens, web_operation))]
pub fn derive_web_lens(input: TokenStream) -> TokenStream {
    // Implementation generates WebLens trait impl
}
```

For simpler integration without proc macros, we can use a registration macro:

```rust
// src/server/registration.rs

/// Register a lens operation for REST API
#[macro_export]
macro_rules! register_rest_operation {
    ($lens:ty, $op_name:literal, $method:ident, $handler:ident, $args:ty, $result:ty) => {
        impl RestOperation<$args, $result> for $lens {
            const OP_NAME: &'static str = $op_name;
            const METHOD: HttpMethod = HttpMethod::$method;
            
            fn execute(&self, args: $args) -> Result<$result, WebError> {
                self.$handler(&args).map_err(|e| WebError::from(e))
            }
        }
    };
}

/// Register a lens operation for WebSocket streaming
#[macro_export]
macro_rules! register_stream_operation {
    ($lens:ty, $op_name:literal, $handler:ident, $args:ty, $item:ty) => {
        impl StreamOperation<$args, $item> for $lens {
            const OP_NAME: &'static str = $op_name;
            
            fn execute_stream(
                &self, 
                args: $args
            ) -> Pin<Box<dyn Stream<Item = Result<$item, WebError>> + Send>> {
                self.$handler(&args)
            }
        }
    };
}
```

## API Endpoints

### Base URL Structure

```
REST:      /api/v1/{lens}/{operation}
WebSocket: /api/v1/ws/{lens}/{operation}
```

### Lens Endpoints

#### 1. Time Lens (`/api/v1/time`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET/POST | `/parse` | Parse time strings | `TimeParseArgs` | `Vec<TimeBgpTime>` |

**Request Example:**
```json
POST /api/v1/time/parse
{
    "times": ["1697043600", "2023-10-11T00:00:00Z"],
    "format": "json"
}
```

**Response Example:**
```json
{
    "data": [
        {
            "unix": 1697043600,
            "rfc3339": "2023-10-11T15:00:00+00:00",
            "human": "about 1 year ago"
        }
    ]
}
```

#### 2. IP Lens (`/api/v1/ip`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET | `/lookup` | Look up IP information | `IpLookupArgs` | `IpInfo` |
| GET | `/lookup/{ip}` | Look up specific IP | path param | `IpInfo` |

**Request Example:**
```
GET /api/v1/ip/lookup?ip=1.1.1.1&simple=false
```

**Response Example:**
```json
{
    "data": {
        "ip": "1.1.1.1",
        "location": "US",
        "network": {
            "asn": 13335,
            "prefix": "1.1.1.0/24",
            "rpki": "valid",
            "name": "CLOUDFLARENET",
            "country": "US"
        }
    }
}
```

#### 3. Country Lens (`/api/v1/country`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET | `/lookup` | Search countries | `query: string` | `Vec<CountryEntry>` |
| GET | `/lookup/{code}` | Lookup by code | path param | `CountryEntry` |
| GET | `/all` | List all countries | - | `Vec<CountryEntry>` |

#### 4. RPKI Lens (`/api/v1/rpki`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET | `/validate` | Validate prefix/ASN | `RpkiValidationArgs` | `RpkiValidity` |
| GET | `/roas` | List ROAs | `RpkiRoaLookupArgs` | `Vec<RpkiRoaEntry>` |
| GET | `/aspas` | List ASPAs | `RpkiAspaLookupArgs` | `Vec<RpkiAspaEntry>` |
| GET | `/summary/{asn}` | Get RPKI summary | `RpkiSummaryArgs` | `RpkiSummary` |

**Request Example:**
```
GET /api/v1/rpki/validate?asn=13335&prefix=1.1.1.0/24
```

**Response Example:**
```json
{
    "data": {
        "state": "valid",
        "covering_roas": [
            {
                "prefix": "1.1.1.0/24",
                "max_length": 24,
                "asn": 13335
            }
        ]
    }
}
```

#### 5. Pfx2as Lens (`/api/v1/pfx2as`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET | `/lookup` | Map prefix to ASN | `Pfx2asLookupArgs` | `Pfx2asResult` |
| GET | `/lookup/{prefix}` | Lookup specific prefix | path param | `Pfx2asResult` |

#### 6. As2org Lens (`/api/v1/as2org`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET/POST | `/search` | Search AS/Org | `As2orgSearchArgs` | `Vec<As2orgSearchResult>` |
| GET | `/lookup/{asn}` | Lookup by ASN | path param | `As2orgSearchResult` |
| GET | `/status` | Get data status | - | `As2orgStatus` |
| POST | `/bootstrap` | Bootstrap data | - | `BootstrapResult` |

**Request Example:**
```json
POST /api/v1/as2org/search
{
    "query": ["cloudflare", "13335"],
    "name_only": false,
    "full_country": true
}
```

#### 7. As2rel Lens (`/api/v1/as2rel`)

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| GET/POST | `/search` | Search relationships | `As2relSearchArgs` | `Vec<As2relSearchResult>` |
| GET | `/lookup/{asn}` | Get ASN relationships | path param | `Vec<As2relSearchResult>` |
| GET | `/pair/{asn1}/{asn2}` | Get pair relationship | path params | `As2relSearchResult` |
| GET | `/status` | Get data status | - | `As2relStatus` |
| POST | `/update` | Update data | - | `UpdateResult` |

#### 8. Parse Lens (`/api/v1/parse`) - Streaming

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| POST | `/file` | Parse MRT file (REST, limited) | `ParseRequest` | `ParseResult` |
| WS | `/stream` | Stream parsed elements | `ParseFilters` | Stream of `BgpElem` |

**WebSocket Protocol:**
```json
// Client -> Server (start parsing)
{
    "action": "start",
    "request_id": "uuid-123",
    "file_path": "https://data.ris.ripe.net/...",
    "filters": {
        "origin_asn": 13335,
        "prefix": "1.1.1.0/24"
    }
}

// Server -> Client (queued - when server is busy)
{"type": "queued", "request_id": "uuid-123", "position": 3, "estimated_wait_seconds": 30}

// Server -> Client (started)
{"type": "started", "request_id": "uuid-123"}

// Server -> Client (streaming results)
{"type": "elem", "data": {...}}
{"type": "elem", "data": {...}}
{"type": "progress", "processed": 1000, "total": null, "rate": 500}
{"type": "complete", "total_elems": 5000, "duration_ms": 2000}

// Client -> Server (cancel)
{"action": "cancel", "request_id": "uuid-123"}

// Server -> Client (cancelled acknowledgment)
{"type": "cancelled", "request_id": "uuid-123", "processed_before_cancel": 2500}

// Server -> Client (on disconnect detection - internal, not sent)
// Server automatically cancels ongoing work and cleans up resources
```

**Disconnection Handling:**
- Server monitors WebSocket connection health via ping/pong
- On client disconnect, server immediately cancels ongoing operations
- Uses `CancellationToken` to propagate cancellation to worker tasks
- All resources (file handles, memory buffers) are cleaned up

#### 9. Search Lens (`/api/v1/search`) - Streaming

| Method | Endpoint | Description | Request | Response |
|--------|----------|-------------|---------|----------|
| POST | `/query` | Query broker | `SearchFilters` | `Vec<BrokerItem>` |
| WS | `/stream` | Stream search results | `SearchFilters` | Stream of `BgpElem` |

**WebSocket Protocol:**
```json
// Client -> Server
{
    "action": "start",
    "request_id": "uuid-456",
    "filters": {
        "start_ts": "2024-01-01T00:00:00Z",
        "end_ts": "2024-01-01T01:00:00Z",
        "prefix": "1.1.1.0/24",
        "collector": "rrc00"
    }
}

// Server -> Client (queued)
{"type": "queued", "request_id": "uuid-456", "position": 1, "estimated_wait_seconds": 60}

// Server -> Client (started)
{"type": "started", "request_id": "uuid-456"}

// Server -> Client
{"type": "broker_result", "files_count": 10, "estimated_duration_seconds": 120}
{"type": "file_start", "file": "...", "index": 1, "total": 10}
{"type": "elem", "data": {...}, "collector": "rrc00"}
{"type": "file_complete", "file": "...", "elems_count": 500}
{"type": "progress", "files_processed": 5, "files_total": 10, "elems_so_far": 2500}
{"type": "complete", "total_files": 10, "total_elems": 5000, "duration_ms": 95000}

// Client -> Server (cancel)
{"action": "cancel", "request_id": "uuid-456"}
```

## Resource Management

### Queue System

For resource-intensive operations (Parse, Search), a queue system prevents server overload:

```rust
// src/server/queue.rs

use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

/// Configuration for the task queue
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Maximum concurrent high-intensity operations
    pub max_concurrent_heavy: usize,
    /// Maximum concurrent medium-intensity operations  
    pub max_concurrent_medium: usize,
    /// Maximum queue size before rejecting requests
    pub max_queue_size: usize,
    /// Request timeout in seconds
    pub request_timeout_secs: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_concurrent_heavy: 2,
            max_concurrent_medium: 10,
            max_queue_size: 100,
            request_timeout_secs: 300, // 5 minutes
        }
    }
}

/// Task queue for managing resource-intensive operations
pub struct TaskQueue {
    config: QueueConfig,
    heavy_semaphore: Arc<Semaphore>,
    medium_semaphore: Arc<Semaphore>,
    queue_size: Arc<AtomicUsize>,
}

impl TaskQueue {
    pub fn new(config: QueueConfig) -> Self {
        Self {
            heavy_semaphore: Arc::new(Semaphore::new(config.max_concurrent_heavy)),
            medium_semaphore: Arc::new(Semaphore::new(config.max_concurrent_medium)),
            queue_size: Arc::new(AtomicUsize::new(0)),
            config,
        }
    }
    
    /// Submit a task to the queue
    /// Returns queue position (0 = running immediately)
    pub async fn submit<F, T>(
        &self,
        intensity: ResourceIntensity,
        cancel_token: CancellationToken,
        task: F,
    ) -> Result<T, WebError>
    where
        F: Future<Output = Result<T, WebError>> + Send,
    {
        // Check queue size
        let current_size = self.queue_size.fetch_add(1, Ordering::SeqCst);
        if current_size >= self.config.max_queue_size {
            self.queue_size.fetch_sub(1, Ordering::SeqCst);
            return Err(WebError::service_unavailable("Queue full, try again later"));
        }
        
        // Acquire semaphore based on intensity
        let permit = match intensity {
            ResourceIntensity::High => self.heavy_semaphore.clone().acquire_owned().await,
            ResourceIntensity::Medium => self.medium_semaphore.clone().acquire_owned().await,
            ResourceIntensity::Low => {
                // Low intensity tasks run immediately
                self.queue_size.fetch_sub(1, Ordering::SeqCst);
                return task.await;
            }
        };
        
        let permit = permit.map_err(|_| WebError::internal("Semaphore closed"))?;
        self.queue_size.fetch_sub(1, Ordering::SeqCst);
        
        // Run task with timeout and cancellation
        let result = tokio::select! {
            result = task => result,
            _ = cancel_token.cancelled() => {
                Err(WebError { code: 499, message: "Client closed request".into(), details: None })
            }
            _ = tokio::time::sleep(Duration::from_secs(self.config.request_timeout_secs)) => {
                Err(WebError { code: 504, message: "Request timeout".into(), details: None })
            }
        };
        
        drop(permit); // Release semaphore
        result
    }
    
    /// Get current queue status
    pub fn status(&self) -> QueueStatus {
        QueueStatus {
            queue_size: self.queue_size.load(Ordering::SeqCst),
            heavy_available: self.heavy_semaphore.available_permits(),
            medium_available: self.medium_semaphore.available_permits(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct QueueStatus {
    pub queue_size: usize,
    pub heavy_available: usize,
    pub medium_available: usize,
}
```

### Connection Management

```rust
// src/server/connection.rs

use tokio_util::sync::CancellationToken;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Manages active WebSocket connections and their associated tasks
pub struct ConnectionManager {
    connections: Arc<RwLock<HashMap<String, ConnectionState>>>,
}

pub struct ConnectionState {
    pub connected_at: chrono::DateTime<chrono::Utc>,
    pub cancel_token: CancellationToken,
    pub active_requests: Vec<String>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Register a new connection
    pub async fn register(&self, connection_id: String) -> CancellationToken {
        let token = CancellationToken::new();
        let state = ConnectionState {
            connected_at: chrono::Utc::now(),
            cancel_token: token.clone(),
            active_requests: Vec::new(),
        };
        self.connections.write().await.insert(connection_id, state);
        token
    }
    
    /// Handle disconnection - cancels all active tasks
    pub async fn disconnect(&self, connection_id: &str) {
        if let Some(state) = self.connections.write().await.remove(connection_id) {
            // Cancel all active tasks for this connection
            state.cancel_token.cancel();
            tracing::info!(
                connection_id = connection_id,
                active_requests = state.active_requests.len(),
                "Connection closed, cancelled active tasks"
            );
        }
    }
    
    /// Add a request to a connection's active list
    pub async fn add_request(&self, connection_id: &str, request_id: String) {
        if let Some(state) = self.connections.write().await.get_mut(connection_id) {
            state.active_requests.push(request_id);
        }
    }
    
    /// Get connection count
    pub async fn connection_count(&self) -> usize {
        self.connections.read().await.len()
    }
}
```

> **Note:** The queue system implementation above provides basic functionality. For production deployments with high traffic, consider:
> - Persistent queue with Redis for crash recovery
> - Distributed queue for multi-server deployments
> - Priority queues for different user tiers
> 
> These advanced features are planned for v2.0.

## Implementation

### Module Structure

```
src/
├── server/
│   ├── mod.rs              # Feature-gated module export
│   ├── traits.rs           # WebLens, StreamLens traits
│   ├── router.rs           # Route registration and dispatch
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── rest.rs         # REST handler implementation
│   │   └── websocket.rs    # WebSocket handler implementation
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── auth.rs         # Optional authentication
│   │   ├── cors.rs         # CORS handling
│   │   └── logging.rs      # Request logging
│   ├── protocol.rs         # WebSocket message types
│   ├── openapi.rs          # OpenAPI spec generation
│   └── server.rs           # Server startup and config
```

### Lens Implementation Pattern

Each lens implements the `WebLens` trait through derive or manual implementation:

```rust
// src/lens/time/mod.rs

use crate::server::{WebLens, WebRequest, WebResponse, OperationMeta, HttpMethod, WebError};

// Args already implement Serialize/Deserialize
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct TimeParseArgs {
    pub times: Vec<String>,
    #[serde(default)]
    pub format: TimeOutputFormat,
}

// Mark as web request
#[cfg(feature = "server")]
impl WebRequest for TimeParseArgs {
    fn validate(&self) -> Result<(), String> {
        // Optional validation
        Ok(())
    }
}

// Mark result as web response
#[cfg(feature = "server")]
impl WebResponse for Vec<TimeBgpTime> {}

// Implement WebLens for TimeLens
#[cfg(feature = "server")]
impl WebLens for TimeLens {
    fn lens_name(&self) -> &'static str {
        "time"
    }
    
    fn operations(&self) -> Vec<OperationMeta> {
        vec![
            OperationMeta {
                name: "parse",
                method: HttpMethod::Post,
                path: "/parse",
                description: "Parse time strings into multiple formats",
                streaming: false,
            }
        ]
    }
    
    fn handle_rest(
        &self,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, WebError> {
        match operation {
            "parse" => {
                let args: TimeParseArgs = serde_json::from_value(payload)
                    .map_err(|e| WebError::bad_request(e.to_string()))?;
                let results = self.parse(&args)
                    .map_err(|e| WebError::internal(e.to_string()))?;
                Ok(serde_json::to_value(results)?)
            }
            _ => Err(WebError::not_found(format!("Unknown operation: {}", operation)))
        }
    }
    
    fn supports_streaming(&self, _operation: &str) -> bool {
        false
    }
}
```

### Automatic Registration

Use the `inventory` crate for automatic lens registration:

```rust
// src/server/registration.rs

use inventory;

/// Lens registration entry
pub struct LensRegistration {
    pub name: &'static str,
    pub factory: fn() -> Box<dyn WebLens>,
}

inventory::collect!(LensRegistration);

/// Register all built-in lenses
#[cfg(feature = "server")]
pub fn register_builtin_lenses() {
    inventory::submit! {
        LensRegistration {
            name: "time",
            factory: || Box::new(TimeLens::new()),
        }
    }
    // ... repeat for other lenses
}

/// Get all registered lenses
pub fn get_all_lenses() -> Vec<Box<dyn WebLens>> {
    inventory::iter::<LensRegistration>
        .into_iter()
        .map(|reg| (reg.factory)())
        .collect()
}
```

### Router Implementation

```rust
// src/server/router.rs

use std::collections::HashMap;
use std::sync::Arc;

pub struct Router {
    lenses: HashMap<String, Arc<dyn WebLens>>,
}

impl Router {
    pub fn new() -> Self {
        let mut router = Self {
            lenses: HashMap::new(),
        };
        
        // Auto-register all lenses
        for lens in get_all_lenses() {
            router.lenses.insert(lens.lens_name().to_string(), Arc::from(lens));
        }
        
        router
    }
    
    pub fn handle_rest(
        &self,
        lens_name: &str,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, WebError> {
        let lens = self.lenses.get(lens_name)
            .ok_or_else(|| WebError::not_found(format!("Unknown lens: {}", lens_name)))?;
        
        lens.handle_rest(operation, payload)
    }
    
    pub fn get_openapi_spec(&self) -> openapi::OpenApi {
        // Generate OpenAPI spec from all registered lenses
        let mut spec = openapi::OpenApi::default();
        
        for lens in self.lenses.values() {
            for op in lens.operations() {
                // Add path to spec
            }
        }
        
        spec
    }
}
```

## Response Format

All REST responses follow a consistent envelope designed for UI consumption:

### Success Response

```json
{
    "success": true,
    "data": { ... },
    "meta": {
        "request_id": "uuid",
        "duration_ms": 42,
        "timestamp": "2024-01-15T10:30:00Z"
    }
}
```

### Success Response with Pagination

```json
{
    "success": true,
    "data": [ ... ],
    "pagination": {
        "page": 1,
        "page_size": 50,
        "total_items": 1250,
        "total_pages": 25,
        "has_next": true,
        "has_prev": false
    },
    "meta": {
        "request_id": "uuid",
        "duration_ms": 42
    }
}
```

### Error Response

```json
{
    "success": false,
    "error": {
        "code": 400,
        "message": "Invalid prefix format",
        "details": {
            "field": "prefix",
            "value": "invalid",
            "hint": "Prefix should be in CIDR notation, e.g., 1.1.1.0/24"
        }
    },
    "meta": {
        "request_id": "uuid"
    }
}
```

### Queue Status Response (for heavy operations)

```json
{
    "success": true,
    "status": "queued",
    "queue": {
        "position": 3,
        "estimated_wait_seconds": 45,
        "request_id": "uuid"
    },
    "meta": {
        "timestamp": "2024-01-15T10:30:00Z"
    }
}
```

### WebSocket Messages

```rust
// src/server/protocol.rs

use serde::{Deserialize, Serialize};

/// Client-to-server WebSocket messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum WsClientMessage {
    /// Start a streaming operation
    #[serde(rename = "start")]
    Start { 
        request_id: String,
        #[serde(flatten)]
        payload: serde_json::Value,
    },
    
    /// Cancel an ongoing operation
    #[serde(rename = "cancel")]
    Cancel { 
        request_id: String 
    },
    
    /// Ping for connection health
    #[serde(rename = "ping")]
    Ping { 
        timestamp: i64 
    },
}

/// Server-to-client WebSocket messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    /// Request has been queued
    #[serde(rename = "queued")]
    Queued { 
        request_id: String,
        position: usize,
        estimated_wait_seconds: u64,
    },
    
    /// Request processing has started
    #[serde(rename = "started")]
    Started { 
        request_id: String 
    },
    
    /// A single result item
    #[serde(rename = "elem")]
    Element { 
        data: serde_json::Value 
    },
    
    /// Batch of result items (for efficiency)
    #[serde(rename = "batch")]
    Batch { 
        data: Vec<serde_json::Value>,
        count: usize,
    },
    
    /// Progress update
    #[serde(rename = "progress")]
    Progress { 
        processed: u64, 
        total: Option<u64>,
        rate: Option<f64>,  // items per second
        eta_seconds: Option<u64>,
        message: Option<String>,
    },
    
    /// Error occurred
    #[serde(rename = "error")]
    Error { 
        request_id: Option<String>,
        code: u16, 
        message: String,
        recoverable: bool,
    },
    
    /// Operation was cancelled
    #[serde(rename = "cancelled")]
    Cancelled {
        request_id: String,
        processed_before_cancel: u64,
    },
    
    /// Operation completed successfully
    #[serde(rename = "complete")]
    Complete { 
        request_id: String,
        total_items: u64,
        duration_ms: u64,
        summary: Option<serde_json::Value>,
    },
    
    /// Pong response
    #[serde(rename = "pong")]
    Pong { 
        timestamp: i64,
        server_time: i64,
    },
}
```

## Feature Flags

```toml
# Cargo.toml

[features]
default = ["cli"]

cli = [...]

server = [
    "dep:axum",
    "dep:tokio",
    "dep:tower",
    "dep:tower-http",
    "dep:inventory",
    "dep:utoipa",  # OpenAPI generation
]

full = ["cli", "server"]
```

## UI Considerations

The API is designed with web and desktop UI applications in mind:

### Pagination Support

All list endpoints support pagination:

```
GET /api/v1/as2org/search?query=cloud&page=1&page_size=50
```

### Sorting and Filtering

List endpoints support sorting:

```
GET /api/v1/as2rel/search?asn=13335&sort=connected_desc&limit=100
```

### Batch Operations

For efficiency, UI can batch multiple lookups:

```json
POST /api/v1/batch
{
    "requests": [
        {"lens": "as2org", "operation": "lookup", "args": {"asn": 13335}},
        {"lens": "as2org", "operation": "lookup", "args": {"asn": 15169}},
        {"lens": "ip", "operation": "lookup", "args": {"ip": "8.8.8.8"}}
    ]
}
```

Response:
```json
{
    "success": true,
    "results": [
        {"index": 0, "success": true, "data": {...}},
        {"index": 1, "success": true, "data": {...}},
        {"index": 2, "success": true, "data": {...}}
    ],
    "meta": {"duration_ms": 150}
}
```

### Export Formats

For data export, support multiple formats:

```
GET /api/v1/as2org/search?query=cloud&format=json
GET /api/v1/as2org/search?query=cloud&format=csv
GET /api/v1/as2org/search?query=cloud&format=xlsx
```

### Server Status Endpoint

For UI health monitoring:

```
GET /api/v1/status
```

Response:
```json
{
    "status": "healthy",
    "version": "0.9.1",
    "uptime_seconds": 86400,
    "lenses": {
        "time": {"status": "ready"},
        "as2org": {"status": "ready", "records": 125000},
        "as2rel": {"status": "ready", "records": 850000},
        "parse": {"status": "ready", "queue_size": 2}
    },
    "queue": {
        "size": 5,
        "heavy_slots_available": 1,
        "medium_slots_available": 8
    }
}
```

### WebSocket Reconnection

UI applications should implement reconnection logic:

1. On disconnect, attempt to reconnect with exponential backoff
2. Server supports resuming operations if client provides `request_id`
3. Server sends missed messages on reconnection (limited buffer)

### Real-time Subscriptions (Future)

For dashboard applications, support subscriptions to data changes:

```json
// Subscribe to AS2Rel updates for specific ASN
{"action": "subscribe", "lens": "as2rel", "filter": {"asn": 13335}}

// Server pushes updates when data changes
{"type": "update", "lens": "as2rel", "data": {...}}
```

> **Note:** Real-time subscriptions are planned for v2.0

## Compile-Time Guarantees

The design ensures compile-time guarantees through:

### 1. Trait Requirements

```rust
/// Every lens must implement the base Lens trait
pub trait Lens: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn category(&self) -> LensCategory;
}

/// When server feature is enabled, lenses must also implement WebLens
#[cfg(feature = "server")]
pub trait LensExt: Lens + WebLens {}
```

### 2. Registration Validation

```rust
/// Compile-time check that all lenses are registered
#[cfg(feature = "server")]
const _: () = {
    // Static assertion that required lenses are registered
    fn assert_lens_registered<T: WebLens>() {}
    
    fn _check() {
        assert_lens_registered::<TimeLens>();
        assert_lens_registered::<IpLens>();
        assert_lens_registered::<CountryLens>();
        assert_lens_registered::<RpkiLens>();
        assert_lens_registered::<Pfx2asLens>();
        assert_lens_registered::<As2orgLens>();
        assert_lens_registered::<As2relLens>();
        assert_lens_registered::<ParseLens>();
        assert_lens_registered::<SearchLens>();
    }
};
```

### 3. Type-Safe Args/Response

```rust
/// Macro to define a lens operation with compile-time type checking
#[macro_export]
macro_rules! define_operation {
    ($lens:ty, $name:ident, $args:ty => $response:ty) => {
        #[cfg(feature = "server")]
        impl $lens {
            pub fn $name##_web(&self, args: $args) -> Result<$response, WebError> {
                self.$name(&args).map_err(Into::into)
            }
        }
    };
}

// Usage
define_operation!(TimeLens, parse, TimeParseArgs => Vec<TimeBgpTime>);
```

## OpenAPI Documentation

Auto-generate OpenAPI spec using `utoipa`:

```rust
// src/server/openapi.rs

use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        time_parse,
        ip_lookup,
        country_lookup,
        rpki_validate,
        rpki_roas,
        pfx2as_lookup,
        as2org_search,
        as2rel_search,
    ),
    components(schemas(
        TimeParseArgs, TimeBgpTime,
        IpLookupArgs, IpInfo,
        CountryEntry,
        RpkiValidationArgs, RpkiValidity,
        Pfx2asLookupArgs, Pfx2asResult,
        As2orgSearchArgs, As2orgSearchResult,
        As2relSearchArgs, As2relSearchResult,
    )),
    tags(
        (name = "time", description = "Time parsing and formatting"),
        (name = "ip", description = "IP information lookup"),
        (name = "country", description = "Country code/name lookup"),
        (name = "rpki", description = "RPKI validation and data"),
        (name = "pfx2as", description = "Prefix-to-ASN mapping"),
        (name = "as2org", description = "AS-to-Organization lookup"),
        (name = "as2rel", description = "AS-level relationships"),
        (name = "parse", description = "MRT file parsing"),
        (name = "search", description = "BGP message search"),
    )
)]
pub struct ApiDoc;
```

## Server Configuration

```rust
// src/server/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host to bind to
    pub host: String,
    
    /// Port to listen on
    pub port: u16,
    
    /// Enable CORS
    pub cors_enabled: bool,
    
    /// Allowed origins for CORS
    pub cors_origins: Vec<String>,
    
    /// Enable API key authentication
    pub auth_enabled: bool,
    
    /// Maximum request body size
    pub max_body_size: usize,
    
    /// WebSocket configuration
    pub websocket: WebSocketConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// Maximum message size
    pub max_message_size: usize,
    
    /// Ping interval in seconds
    pub ping_interval: u64,
    
    /// Maximum concurrent streams per connection
    pub max_concurrent_streams: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            cors_enabled: true,
            cors_origins: vec!["*".to_string()],
            auth_enabled: false,
            max_body_size: 10 * 1024 * 1024, // 10MB
            websocket: WebSocketConfig::default(),
        }
    }
}
```

## Usage Example

### Starting the Server

```rust
// In CLI or as library
use monocle::server::{MonocleServer, ServerConfig};

#[tokio::main]
async fn main() {
    let config = ServerConfig::default();
    let server = MonocleServer::new(config);
    
    // Server auto-registers all lenses
    server.run().await.unwrap();
}
```

### Client Usage

```bash
# REST API
curl http://localhost:8080/api/v1/time/parse \
  -H "Content-Type: application/json" \
  -d '{"times": ["1697043600"]}'

# WebSocket (using wscat)
wscat -c ws://localhost:8080/api/v1/ws/search/stream
> {"action": "start", "filters": {"start_ts": "2024-01-01", "end_ts": "2024-01-02"}}
```

## Implementation Phases

### Phase 1: Core Infrastructure (v0.10)
- [ ] Define `Lens`, `WebLens`, and `StreamLens` traits
- [ ] Implement router and dispatcher
- [ ] Add server feature flag and dependencies
- [ ] Basic queue system with semaphores
- [ ] Connection manager with cancellation

### Phase 2: REST Endpoints (v0.10)
- [ ] Implement WebLens for all standalone lenses (Time, IP, Country, RPKI, Pfx2as)
- [ ] Implement WebLens for database-backed lenses (As2org, As2rel)
- [ ] Add request validation middleware
- [ ] Automatic data initialization on startup
- [ ] Pagination support

### Phase 3: WebSocket Streaming (v0.11)
- [ ] Implement StreamLens for Parse lens
- [ ] Implement StreamLens for Search lens
- [ ] Connection management with proper cleanup
- [ ] Disconnection handling and task cancellation
- [ ] Progress reporting with ETA

### Phase 4: Documentation & UI Support (v0.11)
- [ ] Auto-generate OpenAPI spec
- [ ] Add Swagger UI endpoint
- [ ] Batch operations endpoint
- [ ] Export format support (CSV, XLSX)
- [ ] Server status endpoint

### Phase 5: Production Hardening (v0.12)
- [ ] Optional authentication middleware
- [ ] Rate limiting
- [ ] Performance optimization
- [ ] Metrics and monitoring endpoints

### Future (v2.0)
- [ ] Persistent queue with Redis
- [ ] Real-time subscriptions
- [ ] Distributed deployment support

## Summary

This design provides:

1. **Automatic Endpoint Generation**: Every lens automatically exposes web endpoints through the `WebLens` trait
2. **Type Safety**: Request/response types are compile-time checked via traits
3. **Consistency**: Uniform API patterns and response formats across all lenses
4. **Extensibility**: New lenses automatically get web endpoints by implementing the trait
5. **Streaming Support**: WebSocket for long-running operations with proper cancellation
6. **Resource Management**: Queue system prevents server overload, proper cleanup on disconnect
7. **UI-First Design**: Pagination, batch operations, export formats designed for UI consumption
8. **Documentation**: Auto-generated OpenAPI specs from type definitions

The key insight is leveraging the existing Args structs (which already derive `Serialize`/`Deserialize`) and extending them with the `WebRequest` marker trait, while using a common `Lens` + `WebLens` trait hierarchy that each lens implements to define its operations.

## Related Documents

- [ARCHITECTURE.md](./ARCHITECTURE.md) - Overall system architecture
- [DEVELOPMENT.md](./DEVELOPMENT.md) - Contribution guidelines for adding lenses and web endpoints
- [src/lens/README.md](./src/lens/README.md) - Lens module documentation