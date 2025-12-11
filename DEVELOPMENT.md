# Monocle Development Guide

This guide provides instructions for contributing to monocle, including how to add new lenses, implement web endpoints, and navigate the codebase for bug fixes.

## Table of Contents

1. [Project Structure](#project-structure)
2. [Adding a New Lens](#adding-a-new-lens)
3. [Adding Web/WebSocket Endpoints for a Lens](#adding-webwebsocket-endpoints-for-a-lens)
4. [Finding the Right Place for Fixes](#finding-the-right-place-for-fixes)
5. [Testing Guidelines](#testing-guidelines)
6. [Code Style](#code-style)

## Project Structure

```
src/
├── lib.rs                    # Public API exports
├── config.rs                 # Configuration management
│
├── lens/                     # Business logic layer
│   ├── mod.rs                # Lens module exports
│   ├── traits.rs             # Base Lens trait (when added)
│   ├── as2org/               # Database-backed lens example
│   │   ├── mod.rs            # Lens implementation
│   │   ├── args.rs           # Input argument structs
│   │   └── types.rs          # Output types and enums
│   ├── time/                 # Standalone lens example
│   │   └── mod.rs            # All-in-one implementation
│   └── ...
│
├── database/                 # Data persistence layer
│   ├── core/                 # Connection and schema management
│   ├── session/              # Ephemeral databases (MsgStore)
│   └── monocle/              # Main database repositories
│
├── server/                   # Web server (feature-gated)
│   ├── mod.rs                # Server module exports
│   ├── traits.rs             # WebLens, StreamLens traits
│   ├── router.rs             # Route registration
│   ├── handlers/             # REST and WebSocket handlers
│   └── ...
│
└── bin/
    ├── monocle.rs            # CLI entry point
    └── commands/             # CLI command handlers
```

## Adding a New Lens

Follow these steps to add a new lens to monocle.

### Step 1: Create the Lens Directory

For a lens named `newlens`:

```bash
mkdir -p src/lens/newlens
touch src/lens/newlens/mod.rs
touch src/lens/newlens/args.rs    # Optional: if args are complex
touch src/lens/newlens/types.rs   # Optional: if types are complex
```

For simple lenses, you can put everything in `mod.rs`.

### Step 2: Define Types and Args

Create your argument and result types with proper derives:

```rust
// src/lens/newlens/mod.rs (or args.rs + types.rs for complex lenses)

use serde::{Deserialize, Serialize};

// =============================================================================
// Types
// =============================================================================

/// Result type for the new lens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewLensResult {
    pub field1: String,
    pub field2: u32,
}

/// Output format options
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum NewLensOutputFormat {
    #[default]
    Json,
    Table,
    Simple,
}

// =============================================================================
// Args
// =============================================================================

/// Arguments for new lens operations
/// 
/// This struct works in multiple contexts:
/// - CLI: with clap derives (when `cli` feature is enabled)
/// - REST API: as query parameters or JSON body (via serde)
/// - WebSocket: as JSON message payload (via serde)
/// - Library: constructed programmatically
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct NewLensArgs {
    /// The input query
    #[cfg_attr(feature = "cli", clap(value_name = "QUERY"))]
    pub query: String,

    /// Optional filter
    #[cfg_attr(feature = "cli", clap(short, long))]
    pub filter: Option<String>,

    /// Output format
    #[cfg_attr(feature = "cli", clap(short, long, default_value = "json"))]
    #[serde(default)]
    pub format: NewLensOutputFormat,
}

impl NewLensArgs {
    /// Create new args with query
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Default::default()
        }
    }

    /// Builder: set filter
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Builder: set format
    pub fn with_format(mut self, format: NewLensOutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Validate arguments
    pub fn validate(&self) -> Result<(), String> {
        if self.query.is_empty() {
            return Err("Query cannot be empty".to_string());
        }
        Ok(())
    }
}
```

### Step 3: Implement the Lens

```rust
// src/lens/newlens/mod.rs (continuation)

use anyhow::Result;
use tabled::settings::Style;
use tabled::Table;

// =============================================================================
// Lens
// =============================================================================

/// New lens for [describe what it does]
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::newlens::{NewLens, NewLensArgs};
///
/// let lens = NewLens::new();
/// let args = NewLensArgs::new("example query");
/// let results = lens.lookup(&args)?;
/// ```
pub struct NewLens {
    // Add any state the lens needs
    // For database-backed lenses: db: &'a MonocleDatabase
}

impl NewLens {
    /// Create a new lens instance
    pub fn new() -> Self {
        Self {}
    }

    /// Main operation method
    pub fn lookup(&self, args: &NewLensArgs) -> Result<Vec<NewLensResult>> {
        // Validate args
        args.validate().map_err(|e| anyhow::anyhow!(e))?;
        
        // Implement your logic here
        let results = vec![
            NewLensResult {
                field1: args.query.clone(),
                field2: 42,
            }
        ];
        
        Ok(results)
    }

    /// Format results for display
    pub fn format_results(
        &self,
        results: &[NewLensResult],
        format: &NewLensOutputFormat,
    ) -> String {
        match format {
            NewLensOutputFormat::Json => {
                serde_json::to_string_pretty(results).unwrap_or_default()
            }
            NewLensOutputFormat::Table => {
                Table::new(results).with(Style::rounded()).to_string()
            }
            NewLensOutputFormat::Simple => {
                results
                    .iter()
                    .map(|r| format!("{}: {}", r.field1, r.field2))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

impl Default for NewLens {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_lens_basic() {
        let lens = NewLens::new();
        let args = NewLensArgs::new("test");
        let results = lens.lookup(&args).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_args_validation() {
        let args = NewLensArgs::new("");
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_format_results() {
        let lens = NewLens::new();
        let results = vec![NewLensResult {
            field1: "test".to_string(),
            field2: 42,
        }];
        
        let output = lens.format_results(&results, &NewLensOutputFormat::Json);
        assert!(output.contains("test"));
        assert!(output.contains("42"));
    }
}
```

### Step 4: Export the Lens

Add to `src/lens/mod.rs`:

```rust
pub mod newlens;

// Re-export main types for convenience
pub use newlens::{NewLens, NewLensArgs, NewLensResult, NewLensOutputFormat};
```

### Step 5: Add CLI Command (Optional)

Create `src/bin/commands/newlens.rs`:

```rust
use clap::Args;
use monocle::lens::newlens::{NewLens, NewLensArgs, NewLensOutputFormat};

#[derive(Args)]
pub struct NewLensCliArgs {
    #[clap(flatten)]
    pub args: NewLensArgs,
}

pub fn run(cli_args: NewLensCliArgs, json: bool) {
    let lens = NewLens::new();
    
    match lens.lookup(&cli_args.args) {
        Ok(results) => {
            let format = if json {
                NewLensOutputFormat::Json
            } else {
                cli_args.args.format.clone()
            };
            println!("{}", lens.format_results(&results, &format));
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    }
}
```

Add to `src/bin/commands/mod.rs`:

```rust
pub mod newlens;
```

Add to `src/bin/monocle.rs`:

```rust
use commands::newlens::NewLensCliArgs;

#[derive(Subcommand)]
enum Commands {
    // ... existing commands
    
    /// Description of new lens command
    NewLens(NewLensCliArgs),
}

// In main():
match cli.command {
    // ... existing matches
    Commands::NewLens(args) => commands::newlens::run(args, json),
}
```

### Step 6: Add to Library Exports

Update `src/lib.rs`:

```rust
pub mod lens;

// Re-export for convenience
pub use lens::newlens::{NewLens, NewLensArgs, NewLensResult};
```

## Adding Web/WebSocket Endpoints for a Lens

Once you have a lens, follow these steps to add web endpoints.

### For REST Endpoints

#### Step 1: Implement WebLens Trait

Add to your lens file (feature-gated):

```rust
// src/lens/newlens/mod.rs

#[cfg(feature = "server")]
use crate::server::{
    WebLens, WebRequest, WebResponse, WebError,
    OperationMeta, HttpMethod, ResourceIntensity,
};

// Mark Args as WebRequest
#[cfg(feature = "server")]
impl WebRequest for NewLensArgs {
    fn validate(&self) -> Result<(), String> {
        if self.query.is_empty() {
            return Err("Query cannot be empty".to_string());
        }
        Ok(())
    }
}

// Mark Result as WebResponse
#[cfg(feature = "server")]
impl WebResponse for Vec<NewLensResult> {}

// Implement WebLens
#[cfg(feature = "server")]
impl WebLens for NewLens {
    fn lens_name(&self) -> &'static str {
        "newlens"
    }
    
    fn operations(&self) -> Vec<OperationMeta> {
        vec![
            OperationMeta {
                name: "lookup",
                method: HttpMethod::Get,
                path: "/lookup",
                description: "Look up data using the new lens",
                streaming: false,
                paginated: false,
                resource_intensity: ResourceIntensity::Low,
            }
        ]
    }
    
    fn handle_rest(
        &self,
        operation: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, WebError> {
        match operation {
            "lookup" => {
                let args: NewLensArgs = serde_json::from_value(payload)
                    .map_err(|e| WebError::bad_request(e.to_string()))?;
                
                let results = self.lookup(&args)
                    .map_err(|e| WebError::internal(e.to_string()))?;
                
                serde_json::to_value(results)
                    .map_err(|e| WebError::internal(e.to_string()))
            }
            _ => Err(WebError::not_found(format!("Unknown operation: {}", operation)))
        }
    }
    
    fn supports_streaming(&self, _operation: &str) -> bool {
        false
    }
}
```

#### Step 2: Register the Lens

Add to `src/server/registration.rs`:

```rust
inventory::submit! {
    LensRegistration {
        name: "newlens",
        factory: || Box::new(NewLens::new()),
    }
}
```

### For WebSocket Streaming Endpoints

For lenses that need streaming (like Parse or Search):

```rust
// src/lens/newlens/mod.rs

#[cfg(feature = "server")]
use crate::server::StreamLens;
use futures::Stream;
use std::pin::Pin;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "server")]
impl StreamLens for NewLens {
    fn handle_stream(
        &self,
        operation: &str,
        payload: serde_json::Value,
        cancel_token: CancellationToken,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<serde_json::Value, WebError>> + Send>>, WebError> {
        match operation {
            "stream" => {
                let args: NewLensArgs = serde_json::from_value(payload)
                    .map_err(|e| WebError::bad_request(e.to_string()))?;
                
                // Create an async stream that respects cancellation
                let stream = async_stream::stream! {
                    for i in 0..100 {
                        // Check for cancellation
                        if cancel_token.is_cancelled() {
                            break;
                        }
                        
                        // Yield results
                        let result = NewLensResult {
                            field1: format!("{}-{}", args.query, i),
                            field2: i,
                        };
                        
                        yield Ok(serde_json::to_value(result).unwrap());
                        
                        // Simulate work
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                };
                
                Ok(Box::pin(stream))
            }
            _ => Err(WebError::not_found(format!("Unknown stream operation: {}", operation)))
        }
    }
}

// Update operations() to include streaming operation
#[cfg(feature = "server")]
impl WebLens for NewLens {
    // ... other methods ...
    
    fn operations(&self) -> Vec<OperationMeta> {
        vec![
            OperationMeta {
                name: "lookup",
                method: HttpMethod::Get,
                path: "/lookup",
                description: "Look up data",
                streaming: false,
                paginated: false,
                resource_intensity: ResourceIntensity::Low,
            },
            OperationMeta {
                name: "stream",
                method: HttpMethod::Post, // WebSocket uses POST semantics
                path: "/stream",
                description: "Stream results",
                streaming: true,
                paginated: false,
                resource_intensity: ResourceIntensity::High,
            }
        ]
    }
    
    fn supports_streaming(&self, operation: &str) -> bool {
        operation == "stream"
    }
}
```

## Finding the Right Place for Fixes

Use this guide to locate where to make changes for specific issues:

### Issue Categories

| Issue Type | Location | Files |
|------------|----------|-------|
| Lens logic bug | `src/lens/{lens_name}/mod.rs` | Main lens implementation |
| Argument parsing | `src/lens/{lens_name}/args.rs` | Args struct and validation |
| Output formatting | `src/lens/{lens_name}/mod.rs` | `format_*` methods |
| CLI behavior | `src/bin/commands/{lens_name}.rs` | CLI handler |
| Database queries | `src/database/monocle/{table}.rs` | Repository implementation |
| Schema issues | `src/database/core/schema.rs` | Schema definitions |
| Web API routing | `src/server/router.rs` | Route registration |
| WebSocket protocol | `src/server/protocol.rs` | Message types |
| REST handling | `src/server/handlers/rest.rs` | REST handler |
| WebSocket handling | `src/server/handlers/websocket.rs` | WebSocket handler |
| Configuration | `src/config.rs` | Config loading |

### Finding Code by Symptom

| Symptom | Where to Look |
|---------|---------------|
| Wrong results from lens | `src/lens/{lens}/mod.rs` - main logic |
| CLI flag not working | `src/lens/{lens}/args.rs` - clap attributes |
| JSON output wrong | `src/lens/{lens}/types.rs` - serde attributes |
| Database error | `src/database/monocle/{table}.rs` |
| API returns 404 | `src/server/router.rs` - route registration |
| API returns 500 | `src/lens/{lens}/mod.rs` - error handling |
| WebSocket disconnect | `src/server/handlers/websocket.rs` |

### Lens-Specific Locations

| Lens | Main File | Related Files |
|------|-----------|---------------|
| Time | `src/lens/time/mod.rs` | - |
| IP | `src/lens/ip/mod.rs` | - |
| Country | `src/lens/country.rs` | - |
| RPKI | `src/lens/rpki/mod.rs` | `commons.rs`, `validator.rs` |
| Pfx2as | `src/lens/pfx2as/mod.rs` | - |
| As2org | `src/lens/as2org/mod.rs` | `args.rs`, `types.rs`, `src/database/monocle/as2org.rs` |
| As2rel | `src/lens/as2rel/mod.rs` | `args.rs`, `types.rs`, `src/database/monocle/as2rel.rs` |
| Parse | `src/lens/parse/mod.rs` | - |
| Search | `src/lens/search/mod.rs` | - |

## Testing Guidelines

### Running Tests

```bash
# All tests
cargo test

# Tests for a specific lens
cargo test lens::newlens

# Tests with server feature
cargo test --features server

# Integration tests
cargo test --test '*'
```

### Test Structure

Each lens should have tests covering:

1. **Unit Tests** - In `mod.rs` or separate test file
   - Basic functionality
   - Edge cases
   - Error handling
   - Argument validation

2. **Integration Tests** - In `tests/` directory
   - End-to-end lens operations
   - CLI behavior (if applicable)
   - API endpoints (if applicable)

### Test Template

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Basic functionality
    #[test]
    fn test_basic_operation() {
        let lens = NewLens::new();
        let args = NewLensArgs::new("test");
        let result = lens.lookup(&args);
        assert!(result.is_ok());
    }

    // Edge cases
    #[test]
    fn test_empty_input() {
        let lens = NewLens::new();
        let args = NewLensArgs::new("");
        let result = lens.lookup(&args);
        assert!(result.is_err());
    }

    // Error handling
    #[test]
    fn test_invalid_input() {
        let args = NewLensArgs {
            query: "".to_string(),
            ..Default::default()
        };
        assert!(args.validate().is_err());
    }

    // Format output
    #[test]
    fn test_json_format() {
        let lens = NewLens::new();
        let results = vec![NewLensResult { /* ... */ }];
        let output = lens.format_results(&results, &NewLensOutputFormat::Json);
        assert!(output.starts_with('[') || output.starts_with('{'));
    }

    // Tests requiring network (mark as ignored)
    #[test]
    #[ignore]
    fn test_external_api_call() {
        // Tests that call external APIs
    }
}
```

## Code Style

### Naming Conventions

| Item | Convention | Example |
|------|------------|---------|
| Lens struct | `{Name}Lens` | `TimeLens`, `RpkiLens` |
| Args struct | `{Name}{Op}Args` | `TimeParseArgs`, `RpkiValidationArgs` |
| Result struct | `{Name}{Op}Result` | `As2orgSearchResult` |
| Output format | `{Name}OutputFormat` | `RpkiOutputFormat` |
| Module name | snake_case | `as2org`, `pfx2as` |

### Documentation

Every public item should have documentation:

```rust
/// Brief description of the lens
///
/// Longer description with details about what the lens does,
/// what data sources it uses, etc.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::newlens::{NewLens, NewLensArgs};
///
/// let lens = NewLens::new();
/// let args = NewLensArgs::new("query");
/// let results = lens.lookup(&args)?;
/// ```
pub struct NewLens { ... }
```

### Error Handling

- Use `anyhow::Result` for lens methods
- Provide descriptive error messages
- Use `WebError` for web endpoints with appropriate status codes

```rust
// Good
Err(anyhow!("Invalid prefix format: {}. Expected CIDR notation (e.g., 1.1.1.0/24)", input))

// Bad
Err(anyhow!("invalid input"))
```

### Feature Flags

Always gate server-specific code:

```rust
#[cfg(feature = "server")]
impl WebLens for MyLens { ... }

#[cfg(feature = "cli")]
#[derive(clap::Args)]
pub struct MyArgs { ... }
```

## Checklist for New Lenses

Before submitting a PR for a new lens:

- [ ] Lens struct implemented with `new()` and `Default`
- [ ] Args struct with serde derives and optional clap derives
- [ ] Result types with serde derives
- [ ] Output format enum
- [ ] `format_results()` method for display
- [ ] Unit tests covering basic functionality
- [ ] Documentation with examples
- [ ] Module exported in `src/lens/mod.rs`
- [ ] Types exported in `src/lib.rs`
- [ ] CLI command (if applicable)
- [ ] WebLens implementation (if server feature needed)
- [ ] README updated with new lens description

## Getting Help

- Check existing lens implementations for patterns
- Review `src/lens/README.md` for lens architecture
- Review `WEB_API_DESIGN.md` for web endpoint patterns
- Open an issue for design questions before implementing