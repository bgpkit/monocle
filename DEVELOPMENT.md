# Monocle Development Guide

This guide provides instructions for contributing to monocle, including how to add new lenses and navigate the codebase for bug fixes.

## Table of Contents

1. [Project Structure](#project-structure)
2. [Adding a New Lens](#adding-a-new-lens)
3. [Finding the Right Place for Fixes](#finding-the-right-place-for-fixes)
4. [Testing Guidelines](#testing-guidelines)
5. [Code Style](#code-style)

## Project Structure

```
src/
├── lib.rs                    # Public API exports
├── config.rs                 # Configuration management
│
├── lens/                     # Business logic layer
│   ├── mod.rs                # Lens module exports
│   ├── utils.rs              # Shared utilities (OutputFormat, truncate_name)
│   ├── as2org/               # Database-backed lens example
│   │   ├── mod.rs            # Lens implementation
│   │   ├── args.rs           # Input argument structs
│   │   └── types.rs          # Output types and enums
│   ├── as2rel/               # AS-level relationship lens
│   │   ├── mod.rs
│   │   ├── args.rs
│   │   └── types.rs
│   ├── country.rs            # Country lookup (in-memory)
│   ├── ip/                   # IP information lookup
│   │   └── mod.rs
│   ├── parse/                # MRT file parsing with progress
│   │   └── mod.rs
│   ├── pfx2as/               # Prefix-to-ASN mapping
│   │   └── mod.rs
│   ├── rpki/                 # RPKI validation
│   │   ├── mod.rs
│   │   └── commons.rs
│   ├── search/               # BGP message search with progress
│   │   ├── mod.rs
│   │   └── query_builder.rs
│   └── time/                 # Time parsing lens
│       └── mod.rs
│
├── database/                 # Data persistence layer
│   ├── core/                 # Connection and schema management
│   ├── session/              # Ephemeral databases (MsgStore)
│   └── monocle/              # Main database repositories
│       ├── as2org.rs
│       ├── as2rel.rs
│       ├── rpki.rs
│       └── file_cache.rs
│
└── bin/
    ├── monocle.rs            # CLI entry point
    └── commands/             # CLI command handlers
        ├── as2rel.rs
        ├── config.rs
        ├── country.rs
        ├── database.rs
        ├── ip.rs
        ├── parse.rs
        ├── pfx2as.rs
        ├── rpki.rs
        ├── search.rs
        ├── time.rs
        └── whois.rs
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

// =============================================================================
// Args
// =============================================================================

/// Arguments for new lens operations
/// 
/// This struct works in multiple contexts:
/// - CLI: with clap derives (when `cli` feature is enabled)
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
use crate::lens::utils::OutputFormat;

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
        format: &OutputFormat,
    ) -> String {
        format.format(results)
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
}
```

### Step 4: Export the Lens

Add to `src/lens/mod.rs`:

```rust
pub mod newlens;
```

### Step 5: Add CLI Command (Optional)

Create `src/bin/commands/newlens.rs`:

```rust
use clap::Args;
use monocle::lens::newlens::{NewLens, NewLensArgs};
use monocle::lens::utils::OutputFormat;

#[derive(Args)]
pub struct NewLensCliArgs {
    #[clap(flatten)]
    pub args: NewLensArgs,
}

pub fn run(cli_args: NewLensCliArgs, output_format: OutputFormat) {
    let lens = NewLens::new();
    
    match lens.lookup(&cli_args.args) {
        Ok(results) => {
            println!("{}", lens.format_results(&results, &output_format));
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
    Commands::NewLens(args) => commands::newlens::run(args, output_format),
}
```

### Step 6: Add to Library Exports

Update `src/lib.rs` if you want the lens to be part of the public API.

## Finding the Right Place for Fixes

Use this guide to locate where to make changes for specific issues:

### Issue Categories

| Issue Type | Location | Files |
|------------|----------|-------|
| Lens logic bug | `src/lens/{lens_name}/mod.rs` | Main lens implementation |
| Argument parsing | `src/lens/{lens_name}/args.rs` | Args struct and validation |
| Output formatting | `src/lens/{lens_name}/mod.rs` or `src/lens/utils.rs` | Format methods |
| CLI behavior | `src/bin/commands/{lens_name}.rs` | CLI handler |
| Database queries | `src/database/monocle/{table}.rs` | Repository implementation |
| Schema issues | `src/database/core/schema.rs` | Schema definitions |
| Configuration | `src/config.rs` | Config loading |

### Finding Code by Symptom

| Symptom | Where to Look |
|---------|---------------|
| Wrong results from lens | `src/lens/{lens}/mod.rs` - main logic |
| CLI flag not working | `src/lens/{lens}/args.rs` - clap attributes |
| JSON output wrong | `src/lens/{lens}/types.rs` - serde attributes |
| Database error | `src/database/monocle/{table}.rs` |
| Output format issue | `src/lens/utils.rs` - OutputFormat implementation |

### Lens-Specific Locations

| Lens | Main File | Related Files |
|------|-----------|---------------|
| Time | `src/lens/time/mod.rs` | - |
| IP | `src/lens/ip/mod.rs` | - |
| Country | `src/lens/country.rs` | - |
| RPKI | `src/lens/rpki/mod.rs` | `commons.rs`, `src/database/monocle/rpki.rs` |
| Pfx2as | `src/lens/pfx2as/mod.rs` | `src/database/monocle/file_cache.rs` |
| As2org | `src/lens/as2org/mod.rs` | `args.rs`, `types.rs`, `src/database/monocle/as2org.rs` |
| As2rel | `src/lens/as2rel/mod.rs` | `args.rs`, `types.rs`, `src/database/monocle/as2rel.rs` |
| Parse | `src/lens/parse/mod.rs` | - |
| Search | `src/lens/search/mod.rs` | `query_builder.rs` |

## Testing Guidelines

### Running Tests

```bash
# All tests
cargo test

# Tests for a specific lens
cargo test lens::newlens

# Tests with all features
cargo test --all-features

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

```rust
// Good
Err(anyhow!("Invalid prefix format: {}. Expected CIDR notation (e.g., 1.1.1.0/24)", input))

// Bad
Err(anyhow!("invalid input"))
```

### Feature Flags

Gate CLI-specific code with the `cli` feature:

```rust
#[cfg(feature = "cli")]
#[derive(clap::Args)]
pub struct MyArgs { ... }
```

### Output Formatting

Use the unified `OutputFormat` enum from `src/lens/utils.rs`:

```rust
use crate::lens::utils::OutputFormat;

pub fn format_results(&self, results: &[MyResult], format: &OutputFormat) -> String {
    format.format(results)
}
```

## Checklist for New Lenses

Before submitting a PR for a new lens:

- [ ] Lens struct implemented with `new()` and `Default`
- [ ] Args struct with serde derives and optional clap derives
- [ ] Result types with serde derives
- [ ] `format_results()` method using `OutputFormat`
- [ ] Unit tests covering basic functionality
- [ ] Documentation with examples
- [ ] Module exported in `src/lens/mod.rs`
- [ ] CLI command (if applicable)
- [ ] README updated with new lens description

## Getting Help

- Check existing lens implementations for patterns
- Review `src/lens/README.md` for lens architecture
- Review `ARCHITECTURE.md` for overall project structure
- Open an issue for design questions before implementing