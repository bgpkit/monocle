# Monocle - Agent Instructions

This document provides guidelines for AI coding agents working on the monocle codebase.

## Build, Lint, and Test Commands

### Quick Reference

```bash
# Format code (always run after editing)
cargo fmt

# Build with default features (cli)
cargo build

# Run all tests
cargo test --all-features

# Run clippy with all warnings as errors
cargo clippy --all-features -- -D warnings

# Run a single test by name
cargo test test_name
cargo test lens::time::tests::test_parse_unix

# Run tests for a specific module
cargo test lens::rpki

# Run tests with output visible
cargo test test_name -- --nocapture
```

### CI Checks (run before committing)

The GitHub Actions workflow runs these checks on PRs:

```bash
cargo fmt --check              # Check formatting
cargo clippy --all-features -- -D warnings  # Lint
cargo test --all-features --verbose         # All tests
```

## Code Style Guidelines

### Lints

The crate enforces strict lints in `lib.rs`:

```rust
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
```

Use `?` operator or explicit error handling instead of `.unwrap()` or `.expect()`.

### Import Organization

Group imports in this order, separated by blank lines:
1. Standard library (`std::`)
2. External crates
3. Internal crate modules (`crate::`, `super::`)

### Naming Conventions

| Item | Convention | Example |
|------|------------|---------|
| Lens struct | `{Name}Lens` | `TimeLens`, `RpkiLens`, `InspectLens` |
| Args struct | `{Name}{Op}Args` | `TimeParseArgs`, `RpkiValidationArgs` |
| Result struct | `{Name}{Op}Result` | `As2relSearchResult`, `InspectResult` |
| Module name | snake_case | `as2rel`, `pfx2as`, `inspect` |
| Constants | SCREAMING_SNAKE_CASE | `DEFAULT_RPKI_CACHE_TTL` |

### Error Handling

Use `anyhow::Result` for lens methods with descriptive error messages:

```rust
// Good
Err(anyhow!("Invalid prefix format: {}. Expected CIDR notation (e.g., 1.1.1.0/24)", input))

// Bad
Err(anyhow!("invalid input"))
```

### Feature Gates

Use conditional compilation for feature-specific code:

```rust
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct MyArgs { ... }

// Display (tabled) is always available with lib feature
#[derive(tabled::Tabled)]
pub struct MyResult { ... }
```

Feature flags (3 simple options):
- `lib`: Complete library (database + all lenses + display)
- `server`: WebSocket server (implies lib)
- `cli`: Full CLI binary (implies lib and server)

## Project Structure

```
src/
├── lib.rs              # Public API exports
├── config.rs           # Configuration management
├── database/           # Persistence layer (SQLite)
│   ├── core/           # Connection and schema
│   ├── session/        # Ephemeral databases (MsgStore)
│   └── monocle/        # Main repositories (asinfo, as2rel, rpki, pfx2as)
├── lens/               # Business logic layer
│   ├── utils.rs        # OutputFormat, shared utilities
│   ├── time/           # Time parsing (lens-core)
│   ├── country.rs      # Country lookup (lens-bgpkit)
│   ├── parse/          # MRT parsing (lens-bgpkit)
│   ├── search/         # BGP search (lens-bgpkit)
│   ├── rpki/           # RPKI validation (lens-bgpkit)
│   └── inspect/        # Unified inspection (lens-full)
├── server/             # WebSocket server (cli feature)
│   └── handlers/       # Method handlers
└── bin/
    ├── monocle.rs      # CLI entry point
    └── commands/       # CLI command handlers
```

## Layering Rules

**Repositories** (`database/`): Data access only - CRUD, queries, no business logic
**Lenses** (`lens/`): Business logic, validation, formatting, coordination
**CLI** (`bin/`): Argument parsing, output format selection, error display

## Test Patterns

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operation() {
        let lens = MyLens::new();
        let args = MyArgs::new("test");
        assert!(lens.lookup(&args).is_ok());
    }

    // Tests requiring network - mark as ignored
    #[test]
    #[ignore]
    fn test_external_api() { ... }
}
```

## Git and Commit Guidelines

- Creating new branches is fine, but do NOT commit or push until explicitly asked
- Keep language factual and professional
- Avoid words like "comprehensive", "extensive", "amazing", "powerful", "robust"
- Use objective language: "Added X", "Fixed Y", "Updated Z"
- **Update CHANGELOG.md for every commit** - Add entries to "Unreleased changes" section for:
  - Breaking changes
  - New features
  - Bug fixes
  - Code improvements
- When pushing commits, list all commits first using `git log --oneline origin/[branch]..HEAD` and ask for confirmation

## Common Patterns

### Args with CLI and Library Support

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(clap::Args))]
pub struct MyLensArgs {
    #[cfg_attr(feature = "cli", clap(value_name = "QUERY"))]
    pub query: String,

    #[cfg_attr(feature = "cli", clap(short, long))]
    pub filter: Option<String>,
}

impl MyLensArgs {
    pub fn new(query: impl Into<String>) -> Self {
        Self { query: query.into(), ..Default::default() }
    }
}
```

### Lens Implementation

```rust
pub struct MyLens<'a> {
    db: &'a MonocleDatabase,
}

impl<'a> MyLens<'a> {
    pub fn new(db: &'a MonocleDatabase) -> Self {
        Self { db }
    }

    pub fn lookup(&self, args: &MyLensArgs) -> Result<Vec<MyResult>> {
        Ok(vec![])
    }
}
```

## Related Documentation

- `ARCHITECTURE.md` - Overall project structure
- `DEVELOPMENT.md` - Adding new lenses and fixing bugs
- `src/server/README.md` - WebSocket API specification
- `examples/README.md` - Usage examples by feature tier
