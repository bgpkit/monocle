use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use bgpkit_parser::encoder::MrtUpdatesEncoder;
use bgpkit_parser::BgpElem;
use clap::Args;
use monocle::database::MsgStore;
use monocle::lens::search::SearchFilters;
use monocle::utils::{OrderByField, OrderDirection, OutputFormat, TimestampFormat};
use monocle::MonocleConfig;
use rayon::prelude::*;
use tracing::{info, warn};

use super::elem_format::{
    available_fields_help, format_elem, format_elems_table, get_header, parse_fields, sort_elems,
};

/// Arguments for the Search command
#[derive(Args)]
pub struct SearchArgs {
    /// Dry-run, do not download or parse.
    #[clap(long)]
    pub dry_run: bool,

    /// SQLite output file path
    #[clap(long)]
    pub sqlite_path: Option<PathBuf>,

    /// MRT output file path
    #[clap(long, short = 'M')]
    pub mrt_path: Option<PathBuf>,

    /// SQLite reset database content if exists
    #[clap(long)]
    pub sqlite_reset: bool,

    /// Output matching broker files (URLs) and exit without searching
    #[clap(long)]
    pub broker_files: bool,

    /// Comma-separated list of fields to output
    #[clap(long, short = 'f', value_name = "FIELDS", help = available_fields_help())]
    pub fields: Option<String>,

    /// Order output by field (enables buffering)
    #[clap(long, value_enum)]
    pub order_by: Option<OrderByField>,

    /// Order direction (asc or desc, default: asc)
    #[clap(long, value_enum, default_value = "asc")]
    pub order: OrderDirection,

    /// Timestamp output format for non-JSON output (unix or rfc3339)
    #[clap(long, value_enum, default_value = "unix")]
    pub time_format: TimestampFormat,

    /// Use the default XDG cache directory ($XDG_CACHE_HOME/monocle) for MRT files.
    /// Overridden by --cache-dir if both are specified.
    #[clap(long)]
    pub use_cache: bool,

    /// Override cache directory for downloaded MRT files.
    /// Files are stored as {cache-dir}/{collector}/{path}.
    /// If a file already exists in cache, it will be used instead of downloading.
    #[clap(long)]
    pub cache_dir: Option<PathBuf>,

    /// Filter by AS path regex string
    #[clap(flatten)]
    pub filters: SearchFilters,
}

/// Maximum number of retry attempts (3 attempts total including the first attempt)
const MAX_RETRIES: u32 = 3;

/// Initial retry delay in seconds
const INITIAL_DELAY: u64 = 1;

/// Maximum retry delay in seconds
const MAX_DELAY: u64 = 30;

/// Message types sent through the writer channel
#[derive(Debug)]
enum WriterMessage {
    /// BGP element with its collector ID
    Element(Box<BgpElem>, String),
    /// Signal that a file has been completely processed
    FileComplete,
}

/// Progress update messages for real-time display
#[derive(Debug, Clone)]
enum ProgressUpdate {
    /// A file was completed with message count and success status
    FileComplete { message_count: u32, success: bool },
    /// A new page started processing
    PageStarted { page_num: i64, timestamp: String },
}

/// Structure to track failed processing attempts for retry mechanism
#[derive(Debug, Clone)]
struct FailedItem {
    item: bgpkit_broker::BrokerItem,
    attempt_count: u32,
    last_error: String,
}

impl FailedItem {
    fn new(item: bgpkit_broker::BrokerItem, error: String) -> Self {
        Self {
            item,
            attempt_count: 1,
            last_error: error,
        }
    }

    fn next_delay(&self) -> Duration {
        let base_delay = INITIAL_DELAY * 2_u64.pow(self.attempt_count - 1);
        let delay = std::cmp::min(base_delay, MAX_DELAY);
        Duration::from_secs(delay)
    }

    fn should_retry(&self) -> bool {
        self.attempt_count < MAX_RETRIES
    }

    fn increment_attempt(&mut self, error: String) {
        self.attempt_count += 1;
        self.last_error = error;
    }
}

/// Constructs the local cache path for a given URL and collector.
///
/// The path structure is: {cache_dir}/{collector}/{path_after_domain}
///
/// Examples:
/// - RIPE RIS: https://data.ris.ripe.net/rrc00/2024.01/updates.20240101.0000.gz
///   -> {cache_dir}/rrc00/2024.01/updates.20240101.0000.gz
/// - RouteViews: http://archive.routeviews.org/route-views6/bgpdata/2024.01/UPDATES/updates.bz2
///   -> {cache_dir}/route-views6/bgpdata/2024.01/UPDATES/updates.bz2
fn url_to_cache_path(cache_dir: &Path, collector: &str, url: &str) -> Option<PathBuf> {
    // Extract path from URL by finding the first '/' after the protocol
    let path = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.find('/').map(|idx| &rest[idx..]))?;

    let path = path.trim_start_matches('/');

    // Check if path starts with the collector
    let relative_path = if path.starts_with(collector) {
        path.to_string()
    } else {
        // Prepend collector to path
        format!("{}/{}", collector, path)
    };

    Some(cache_dir.join(relative_path))
}

/// Downloads a file to the cache directory with .partial extension during download,
/// then renames to the final path on success.
fn download_to_cache(url: &str, cache_path: &Path) -> Result<(), anyhow::Error> {
    // Create parent directories if they don't exist
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Build the .partial path
    let partial_path = {
        let file_name = cache_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        cache_path.with_file_name(format!("{}.partial", file_name))
    };

    // Download to .partial file
    oneio::download(url, partial_path.to_str().unwrap_or_default(), None)?;

    // Rename .partial to final path
    std::fs::rename(&partial_path, cache_path)?;

    Ok(())
}

/// Validates that the cache directory is accessible (can be created and written to).
/// Returns an error message if validation fails.
fn validate_cache_dir(cache_dir: &Path) -> Result<(), String> {
    // Try to create the directory if it doesn't exist
    if let Err(e) = std::fs::create_dir_all(cache_dir) {
        return Err(format!(
            "Cannot create cache directory '{}': {}",
            cache_dir.display(),
            e
        ));
    }

    // Check if we can write to the directory by creating a test file
    let test_file = cache_dir.join(".monocle_cache_test");
    if let Err(e) = std::fs::write(&test_file, b"test") {
        return Err(format!(
            "Cannot write to cache directory '{}': {}",
            cache_dir.display(),
            e
        ));
    }

    // Clean up test file
    let _ = std::fs::remove_file(&test_file);

    Ok(())
}

// =============================================================================
// Broker Cache Module
// =============================================================================

mod broker_cache {
    use bgpkit_broker::BrokerItem;
    use rusqlite::{params, Connection};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::path::Path;

    /// Minimum age (in seconds) for query end time to be considered cacheable.
    /// Queries with end_time within this window are always fetched fresh.
    const CACHE_STALENESS_THRESHOLD_SECS: i64 = 2 * 60 * 60; // 2 hours

    /// Opens or creates the broker cache database.
    pub fn open_cache_db(cache_dir: &Path) -> Result<Connection, rusqlite::Error> {
        let db_path = cache_dir.join("broker-cache.sqlite3");
        let conn = Connection::open(&db_path)?;

        // Create tables if they don't exist
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS broker_items (
                id INTEGER PRIMARY KEY,
                ts_start INTEGER NOT NULL,
                ts_end INTEGER NOT NULL,
                collector_id TEXT NOT NULL,
                data_type TEXT NOT NULL,
                url TEXT NOT NULL UNIQUE,
                rough_size INTEGER NOT NULL,
                exact_size INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_broker_items_time 
                ON broker_items(ts_start, ts_end);
            CREATE INDEX IF NOT EXISTS idx_broker_items_collector 
                ON broker_items(collector_id);
            CREATE INDEX IF NOT EXISTS idx_broker_items_data_type 
                ON broker_items(data_type);

            CREATE TABLE IF NOT EXISTS cached_queries (
                id INTEGER PRIMARY KEY,
                query_hash TEXT NOT NULL UNIQUE,
                ts_start INTEGER NOT NULL,
                ts_end INTEGER NOT NULL,
                collector TEXT,
                project TEXT,
                data_type TEXT,
                cached_at INTEGER NOT NULL
            );
            "#,
        )?;

        Ok(conn)
    }

    /// Computes a hash for the query parameters.
    pub fn compute_query_hash(
        ts_start: i64,
        ts_end: i64,
        collector: Option<&str>,
        project: Option<&str>,
        data_type: Option<&str>,
    ) -> String {
        let mut hasher = DefaultHasher::new();

        // Round timestamps to nearest 5 minutes for better cache hits
        let ts_start_rounded = (ts_start / 300) * 300;
        let ts_end_rounded = (ts_end / 300) * 300;

        ts_start_rounded.hash(&mut hasher);
        ts_end_rounded.hash(&mut hasher);
        collector.unwrap_or("*").hash(&mut hasher);
        project.unwrap_or("*").hash(&mut hasher);
        data_type.unwrap_or("*").hash(&mut hasher);

        format!("{:016x}", hasher.finish())
    }

    /// Checks if the query end time is old enough to be cacheable.
    pub fn is_cacheable_query(ts_end: i64) -> bool {
        let now = chrono::Utc::now().timestamp();
        (now - ts_end) >= CACHE_STALENESS_THRESHOLD_SECS
    }

    /// Checks if a query has been cached.
    pub fn is_query_cached(conn: &Connection, query_hash: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM cached_queries WHERE query_hash = ?1",
            params![query_hash],
            |_| Ok(()),
        )
        .is_ok()
    }

    /// Retrieves cached broker items matching the query parameters.
    pub fn get_cached_items(
        conn: &Connection,
        ts_start: i64,
        ts_end: i64,
        collector: Option<&str>,
        project: Option<&str>,
        data_type: Option<&str>,
    ) -> Result<Vec<BrokerItem>, rusqlite::Error> {
        // Build dynamic query based on filters
        let mut sql = String::from(
            "SELECT ts_start, ts_end, collector_id, data_type, url, rough_size, exact_size 
             FROM broker_items 
             WHERE ts_start >= ?1 AND ts_end <= ?2",
        );
        let mut param_idx = 3;

        if collector.is_some() {
            sql.push_str(&format!(" AND collector_id = ?{}", param_idx));
            param_idx += 1;
        }
        if data_type.is_some() {
            sql.push_str(&format!(" AND data_type = ?{}", param_idx));
            // param_idx += 1; // Unused after this
        }

        sql.push_str(" ORDER BY ts_start, data_type, collector_id");

        let mut stmt = conn.prepare(&sql)?;

        // Build params dynamically
        let rows = match (collector, data_type) {
            (Some(c), Some(d)) => stmt.query_map(params![ts_start, ts_end, c, d], row_to_item)?,
            (Some(c), None) => stmt.query_map(params![ts_start, ts_end, c], row_to_item)?,
            (None, Some(d)) => stmt.query_map(params![ts_start, ts_end, d], row_to_item)?,
            (None, None) => stmt.query_map(params![ts_start, ts_end], row_to_item)?,
        };

        // Note: project filter is not stored in broker_items, so we filter in memory if needed
        let items: Vec<BrokerItem> = rows.filter_map(|r| r.ok()).collect();

        // Filter by project if specified (project info would need to be derived from URL or collector)
        // For now, we don't filter by project since it's implicit in collector_id
        let _ = project; // Acknowledge unused parameter

        Ok(items)
    }

    fn row_to_item(row: &rusqlite::Row) -> Result<BrokerItem, rusqlite::Error> {
        let ts_start_secs: i64 = row.get(0)?;
        let ts_end_secs: i64 = row.get(1)?;

        // Convert timestamps to NaiveDateTime via DateTime
        let ts_start = chrono::DateTime::from_timestamp(ts_start_secs, 0)
            .map(|dt| dt.naive_utc())
            .unwrap_or_default();
        let ts_end = chrono::DateTime::from_timestamp(ts_end_secs, 0)
            .map(|dt| dt.naive_utc())
            .unwrap_or_default();

        Ok(BrokerItem {
            ts_start,
            ts_end,
            collector_id: row.get(2)?,
            data_type: row.get(3)?,
            url: row.get(4)?,
            rough_size: row.get(5)?,
            exact_size: row.get(6)?,
        })
    }

    /// Stores broker items in the cache.
    pub fn store_items(conn: &Connection, items: &[BrokerItem]) -> Result<(), rusqlite::Error> {
        let mut stmt = conn.prepare(
            "INSERT OR REPLACE INTO broker_items 
             (ts_start, ts_end, collector_id, data_type, url, rough_size, exact_size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;

        for item in items {
            stmt.execute(params![
                item.ts_start.and_utc().timestamp(),
                item.ts_end.and_utc().timestamp(),
                &item.collector_id,
                &item.data_type,
                &item.url,
                item.rough_size,
                item.exact_size,
            ])?;
        }

        Ok(())
    }

    /// Records that a query has been cached.
    pub fn record_cached_query(
        conn: &Connection,
        query_hash: &str,
        ts_start: i64,
        ts_end: i64,
        collector: Option<&str>,
        project: Option<&str>,
        data_type: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        conn.execute(
            "INSERT OR REPLACE INTO cached_queries 
             (query_hash, ts_start, ts_end, collector, project, data_type, cached_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                query_hash,
                ts_start,
                ts_end,
                collector,
                project,
                data_type,
                chrono::Utc::now().timestamp(),
            ],
        )?;

        Ok(())
    }
}

/// Fetches broker items, using cache if available and appropriate.
/// Returns (items, used_cache) tuple.
fn fetch_broker_items_cached(
    filters: &SearchFilters,
    cache_dir: Option<&Path>,
) -> Result<(Vec<bgpkit_broker::BrokerItem>, bool), String> {
    // Get time range from filters
    let (ts_start, ts_end) = filters
        .parse_filters
        .parse_start_end_strings()
        .map_err(|e| format!("Failed to parse time range: {}", e))?;

    // Extract filter parameters
    let collector = filters.collector.as_deref();
    let project = filters.project.as_deref();
    let data_type = match filters.dump_type {
        monocle::lens::search::SearchDumpType::Updates => Some("updates"),
        monocle::lens::search::SearchDumpType::Rib => Some("rib"),
        monocle::lens::search::SearchDumpType::RibUpdates => None,
    };

    // Check if we should use cache
    if let Some(cache_dir) = cache_dir {
        let is_cacheable = broker_cache::is_cacheable_query(ts_end);

        if is_cacheable {
            // Try to open cache database
            if let Ok(conn) = broker_cache::open_cache_db(cache_dir) {
                let query_hash = broker_cache::compute_query_hash(
                    ts_start, ts_end, collector, project, data_type,
                );

                // Check if query is cached
                if broker_cache::is_query_cached(&conn, &query_hash) {
                    // Fetch from cache
                    match broker_cache::get_cached_items(
                        &conn, ts_start, ts_end, collector, project, data_type,
                    ) {
                        Ok(items) if !items.is_empty() => {
                            return Ok((items, true));
                        }
                        _ => {
                            // Cache miss or empty - fall through to API query
                        }
                    }
                }
            }
        }
    }

    // Query broker API
    let broker = filters
        .build_broker()
        .map_err(|e| format!("Failed to build broker: {}", e))?;

    let items = broker
        .query()
        .map_err(|e| format!("Failed to query broker: {}", e))?;

    // Store in cache if appropriate
    if let Some(cache_dir) = cache_dir {
        let is_cacheable = broker_cache::is_cacheable_query(ts_end);

        if is_cacheable {
            if let Ok(conn) = broker_cache::open_cache_db(cache_dir) {
                // Store items
                if let Err(e) = broker_cache::store_items(&conn, &items) {
                    // Log warning but don't fail
                    tracing::warn!("Failed to cache broker items: {}", e);
                }

                // Record the query as cached
                let query_hash = broker_cache::compute_query_hash(
                    ts_start, ts_end, collector, project, data_type,
                );
                if let Err(e) = broker_cache::record_cached_query(
                    &conn,
                    &query_hash,
                    ts_start,
                    ts_end,
                    collector,
                    project,
                    data_type,
                ) {
                    tracing::warn!("Failed to record cached query: {}", e);
                }
            }
        }
    }

    Ok((items, false))
}

pub fn run(config: &MonocleConfig, args: SearchArgs, output_format: OutputFormat) {
    let SearchArgs {
        dry_run,
        sqlite_path,
        mrt_path,
        sqlite_reset,
        broker_files,
        fields: fields_arg,
        order_by,
        order,
        time_format,
        use_cache,
        cache_dir,
        filters,
    } = args;

    let cache_dir = match cache_dir {
        Some(cache_dir) => Some(cache_dir),
        None => use_cache.then(|| PathBuf::from(config.cache_dir())),
    };

    // Parse and validate fields (true = search command, include collector in defaults)
    let fields = match parse_fields(&fields_arg, true) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = filters.validate() {
        eprintln!("ERROR: {e}");
        return;
    }

    // Validate cache directory access upfront if caching is enabled
    if let Some(ref cache_dir) = cache_dir {
        if let Err(e) = validate_cache_dir(cache_dir) {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    }

    let mut sqlite_path_str = "".to_string();
    let sqlite_db = sqlite_path.and_then(|p| {
        p.to_str().map(|s| {
            sqlite_path_str = s.to_string();
            match MsgStore::new_from_option(&Some(sqlite_path_str.clone()), sqlite_reset) {
                Ok(store) => store,
                Err(e) => {
                    eprintln!("Failed to create SQLite store: {}", e);
                    std::process::exit(1);
                }
            }
        })
    });
    let mrt_path = mrt_path.and_then(|p| p.to_str().map(|s| s.to_string()));
    let show_progress = sqlite_db.is_some() || mrt_path.is_some();

    // Create base broker for pagination
    let base_broker = match filters.build_broker() {
        Ok(broker) => broker,
        Err(e) => {
            eprintln!("Failed to create broker: {}", e);
            std::process::exit(1);
        }
    };

    if broker_files {
        // Output all matching broker files and exit without searching
        let (items, used_cache) = if cache_dir.is_some() {
            match fetch_broker_items_cached(&filters, cache_dir.as_deref()) {
                Ok(result) => result,
                Err(e) => {
                    eprintln!("Failed to query broker: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            // Cache disabled, query broker directly
            match base_broker.query() {
                Ok(items) => (items, false),
                Err(e) => {
                    eprintln!("Failed to query broker: {}", e);
                    std::process::exit(1);
                }
            }
        };

        if used_cache {
            info!("Using cached broker results ({} items)", items.len());
        }

        match output_format {
            OutputFormat::Json => match serde_json::to_string(&items) {
                Ok(json_str) => println!("{}", json_str),
                Err(e) => eprintln!("error serializing: {}", e),
            },
            OutputFormat::JsonPretty => match serde_json::to_string_pretty(&items) {
                Ok(json_str) => println!("{}", json_str),
                Err(e) => eprintln!("error serializing: {}", e),
            },
            OutputFormat::JsonLine => {
                for item in &items {
                    match serde_json::to_string(item) {
                        Ok(json_str) => println!("{}", json_str),
                        Err(e) => eprintln!("error serializing: {}", e),
                    }
                }
            }
            _ => {
                for item in &items {
                    println!("{}", item.url);
                }
            }
        }
        return;
    }

    if dry_run {
        // For dry run, get first page to show what would be processed
        let items = match base_broker.clone().page(1).query_single_page() {
            Ok(items) => items,
            Err(e) => {
                eprintln!("Failed to query broker for dry run: {}", e);
                std::process::exit(1);
            }
        };

        let total_size: i64 = items.iter().map(|x| x.rough_size).sum();
        if output_format.is_json() {
            let dry_run_info = serde_json::json!({
                "dry_run": true,
                "first_page_files": items.len(),
                "first_page_bytes": total_size,
                "note": "will process all pages with ~1000 files each"
            });
            match output_format {
                OutputFormat::JsonPretty => println!(
                    "{}",
                    serde_json::to_string_pretty(&dry_run_info).unwrap_or_default()
                ),
                _ => println!(
                    "{}",
                    serde_json::to_string(&dry_run_info).unwrap_or_default()
                ),
            }
        } else {
            eprintln!(
                "First page: {} files, {} bytes (will process all pages with ~1000 files each)",
                items.len(),
                total_size
            );
        }
        return;
    }

    let (sender, receiver): (Sender<WriterMessage>, Receiver<WriterMessage>) = channel();
    // Single progress channel for all updates
    let (progress_sender, progress_receiver): (Sender<ProgressUpdate>, Receiver<ProgressUpdate>) =
        channel();

    // Clone fields for the writer thread
    let fields_for_writer: Vec<&'static str> = fields.clone();
    let is_table_format = output_format == OutputFormat::Table;
    // Determine if we need buffering for sorting (in addition to table format)
    let needs_sorting = order_by.is_some();
    let needs_buffering = is_table_format || needs_sorting;
    // Clone ordering parameters for writer thread
    let order_by_for_writer = order_by;
    let order_for_writer = order;
    // Clone time format for writer thread
    let time_format_for_writer = time_format;

    // dedicated thread for handling output of results
    let writer_thread = thread::spawn(move || {
        let display_stdout = sqlite_db.is_none() && mrt_path.is_none();
        let mut mrt_writer = match mrt_path {
            Some(p) => match oneio::get_writer(p.as_str()) {
                Ok(writer) => Some((MrtUpdatesEncoder::new(), writer)),
                Err(e) => {
                    eprintln!("Failed to create MRT writer: {}", e);
                    None
                }
            },
            None => None,
        };

        let mut current_file_cache = vec![];
        let mut total_msg_count = 0;
        let mut header_printed = false;
        // Buffer for Table format or sorted output - collects all elements before display
        let mut output_buffer: Vec<(BgpElem, Option<String>)> = Vec::new();

        for msg in receiver {
            match msg {
                WriterMessage::Element(elem, collector) => {
                    total_msg_count += 1;

                    if display_stdout {
                        // For Table format or when sorting is needed, buffer all elements
                        if needs_buffering {
                            output_buffer.push((*elem, Some(collector)));
                            continue;
                        }

                        // Print header for markdown formats on first element
                        if !header_printed {
                            if let Some(header) = get_header(output_format, &fields_for_writer) {
                                println!("{header}");
                            }
                            header_printed = true;
                        }
                        if let Some(output_str) = format_elem(
                            &elem,
                            output_format,
                            &fields_for_writer,
                            Some(&collector),
                            time_format_for_writer,
                        ) {
                            println!("{output_str}");
                        }
                        continue;
                    }

                    current_file_cache.push((*elem, collector));
                }
                WriterMessage::FileComplete => {
                    // Commit current file's data to SQLite
                    if !current_file_cache.is_empty() {
                        if let Some(db) = &sqlite_db {
                            if let Err(e) = db.insert_elems(&current_file_cache) {
                                eprintln!("Failed to insert elements to database: {}", e);
                            }
                        }
                        if let Some((encoder, _writer)) = &mut mrt_writer {
                            for (elem, _) in &current_file_cache {
                                encoder.process_elem(elem);
                            }
                        }
                        current_file_cache.clear();
                    }
                }
            }
        }

        // Handle any remaining data in cache (in case last file didn't send FileComplete)
        if !current_file_cache.is_empty() {
            if let Some(db) = &sqlite_db {
                if let Err(e) = db.insert_elems(&current_file_cache) {
                    eprintln!("Failed to insert elements to database: {}", e);
                }
            }
            if let Some((encoder, _writer)) = &mut mrt_writer {
                for (elem, _) in &current_file_cache {
                    encoder.process_elem(elem);
                }
            }
        }

        if let Some((encoder, writer)) = &mut mrt_writer {
            let bytes = encoder.export_bytes();
            if let Err(e) = writer.write_all(&bytes) {
                eprintln!("Failed to write MRT data: {}", e);
            }
        }
        drop(mrt_writer);

        // For buffered output (Table format or sorted), process at the end
        if display_stdout && needs_buffering && !output_buffer.is_empty() {
            // Sort if ordering is requested
            if let Some(order_field) = order_by_for_writer {
                sort_elems(&mut output_buffer, order_field, order_for_writer);
            }

            // Output based on format
            if is_table_format {
                println!(
                    "{}",
                    format_elems_table(&output_buffer, &fields_for_writer, time_format_for_writer)
                );
            } else {
                // Print header for markdown format
                if let Some(header) = get_header(output_format, &fields_for_writer) {
                    println!("{header}");
                }

                // Output sorted elements
                for (elem, collector) in &output_buffer {
                    if let Some(output_str) = format_elem(
                        elem,
                        output_format,
                        &fields_for_writer,
                        collector.as_deref(),
                        time_format_for_writer,
                    ) {
                        println!("{output_str}");
                    }
                }
            }
        }

        if !display_stdout {
            eprintln!("found {total_msg_count} messages, written into file {sqlite_path_str}");
        }
    });

    // Setup spinner for paginated processing
    let pb = if show_progress {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_message("Processed 0 files, found 0 messages");
        pb.enable_steady_tick(Duration::from_millis(100));
        Some(pb)
    } else {
        None
    };

    // Simplified progress thread with single channel
    let pb_for_updates = pb.clone();
    let progress_thread = thread::spawn(move || {
        let mut files_processed: u64 = 0;
        let mut total_messages: u64 = 0;
        let mut succeeded_files: u64 = 0;
        let mut failed_files: u64 = 0;
        let mut current_page: i64 = 1;
        let mut current_timestamp = String::new();

        for update in progress_receiver {
            match update {
                ProgressUpdate::FileComplete {
                    message_count,
                    success,
                } => {
                    files_processed += 1;
                    total_messages += message_count as u64;
                    if success {
                        succeeded_files += 1;
                    } else {
                        failed_files += 1;
                    }
                }
                ProgressUpdate::PageStarted {
                    page_num,
                    timestamp,
                } => {
                    current_page = page_num;
                    current_timestamp = timestamp;
                }
            }

            // Update progress display
            if let Some(ref pb) = pb_for_updates {
                let page_info = if current_timestamp.is_empty() {
                    format!(
                        " | Page {} (succeeded: {}, failed: {})",
                        current_page, succeeded_files, failed_files
                    )
                } else {
                    format!(
                        " | Page {} (succeeded: {}, failed: {}) {}",
                        current_page, succeeded_files, failed_files, current_timestamp
                    )
                };

                pb.set_message(format!(
                    "Processed {} files, found {} messages{}",
                    files_processed, total_messages, page_info
                ));
            }
        }
    });

    // Create shared structure to collect failed items
    let failed_items = Arc::new(Mutex::new(Vec::<FailedItem>::new()));
    let failed_items_clone = Arc::clone(&failed_items);

    // Only use broker cache when caching is enabled; otherwise use pagination
    let (all_items, used_broker_cache) = if cache_dir.is_some() {
        match fetch_broker_items_cached(&filters, cache_dir.as_deref()) {
            Ok((items, used_cache)) => (Some(items), used_cache),
            Err(e) => {
                // If cache fetch failed, log and continue with pagination
                warn!("Broker cache query failed, using pagination: {}", e);
                (None, false)
            }
        }
    } else {
        // Cache disabled, use pagination (original behavior)
        (None, false)
    };

    if used_broker_cache {
        info!(
            "Using cached broker results ({} items)",
            all_items.as_ref().map(|v| v.len()).unwrap_or(0)
        );
    }

    // Determine if we're using cached items or pagination
    let use_pagination = all_items.is_none();
    let mut page = 1i64;
    let items_iter: Box<dyn Iterator<Item = Vec<bgpkit_broker::BrokerItem>>> =
        if let Some(items) = all_items {
            // Process all items in batches of 1000 (same as pagination page size)
            let total_items = items.len();
            let batches: Vec<Vec<bgpkit_broker::BrokerItem>> =
                items.chunks(1000).map(|c| c.to_vec()).collect();
            info!(
                "Processing {} cached items in {} batches",
                total_items,
                batches.len()
            );
            Box::new(batches.into_iter())
        } else {
            // Use pagination iterator
            Box::new(std::iter::from_fn({
                let broker = base_broker.clone();
                let mut current_page = 1i64;
                move || {
                    let items = broker.clone().page(current_page).query_single_page().ok()?;
                    if items.is_empty() {
                        return None;
                    }
                    current_page += 1;
                    Some(items)
                }
            }))
        };

    for items in items_iter {
        if items.is_empty() {
            info!("Reached empty batch, finishing");
            break;
        }

        let page_size = items.len();

        // Send page started update to progress thread
        let time_info = if let Some(first_item) = items.first() {
            format!("@ {}", first_item.ts_start.format("%Y-%m-%d %H:%M UTC"))
        } else {
            String::new()
        };

        if progress_sender
            .send(ProgressUpdate::PageStarted {
                page_num: page,
                timestamp: time_info.clone(),
            })
            .is_err()
        {
            // Progress thread may have ended, continue
        }

        if !show_progress {
            let source = if use_pagination { "page" } else { "batch" };
            info!(
                "Starting {} {} ({} files){}",
                source, page, page_size, time_info
            );
            info!("Processing {} {} with {} items", source, page, page_size);
        }

        // Process this page's items using existing parallel logic
        let progress_sender_clone = progress_sender.clone();
        let cache_dir_clone = cache_dir.clone();

        items.into_par_iter().for_each_with(
            (
                sender.clone(),
                progress_sender_clone,
                failed_items_clone.clone(),
            ),
            |(s, progress_sender, failed_items), item| {
                let url = item.url.clone();
                let collector = item.collector_id.clone();

                // Determine the file path to parse (local cache or remote URL)
                let file_path = if let Some(ref cache_dir) = cache_dir_clone {
                    let cache_path = match url_to_cache_path(cache_dir, &collector, &url) {
                        Some(p) => p,
                        None => {
                            let error_msg = format!("Failed to construct cache path for {}", url);
                            if let Ok(mut failed) = failed_items.lock() {
                                failed.push(FailedItem::new(item, error_msg));
                            }
                            let _ = progress_sender.send(ProgressUpdate::FileComplete {
                                message_count: 0,
                                success: false,
                            });
                            return;
                        }
                    };

                    // Check if file exists in cache, download if not
                    if !cache_path.exists() {
                        if let Err(e) = download_to_cache(&url, &cache_path) {
                            let error_msg = format!("Failed to download {} to cache: {}", url, e);
                            if let Ok(mut failed) = failed_items.lock() {
                                failed.push(FailedItem::new(item, error_msg));
                            }
                            let _ = progress_sender.send(ProgressUpdate::FileComplete {
                                message_count: 0,
                                success: false,
                            });
                            return;
                        }
                    }
                    cache_path.to_string_lossy().to_string()
                } else {
                    url.clone()
                };

                if !show_progress {
                    info!("start parsing {}", file_path.as_str());
                }

                let parser = match filters.to_parser(file_path.as_str()) {
                    Ok(p) => p,
                    Err(e) => {
                        let error_msg = format!("Failed to parse {}: {}", file_path.as_str(), e);
                        if !show_progress {
                            eprintln!("{}", error_msg);
                        }

                        // If using cache and parse failed, delete the cached file (might be corrupted)
                        if cache_dir_clone.is_some() {
                            let _ = std::fs::remove_file(&file_path);
                        }

                        // Store failed item for retry
                        if let Ok(mut failed) = failed_items.lock() {
                            failed.push(FailedItem::new(item, error_msg));
                        }
                        // Send failure progress update
                        if progress_sender
                            .send(ProgressUpdate::FileComplete {
                                message_count: 0,
                                success: false,
                            })
                            .is_err()
                        {
                            // Progress thread may have ended, ignore
                        }
                        return;
                    }
                };

                let mut elems_count = 0;
                for elem in parser {
                    if s.send(WriterMessage::Element(Box::new(elem), collector.clone()))
                        .is_err()
                    {
                        // Channel closed, break out
                        break;
                    }
                    elems_count += 1;
                }

                // Send file completion signal to trigger per-file commit
                if s.send(WriterMessage::FileComplete).is_err() {
                    // Channel closed, ignore
                }

                // Send success progress update
                if progress_sender
                    .send(ProgressUpdate::FileComplete {
                        message_count: elems_count,
                        success: true,
                    })
                    .is_err()
                {
                    // Progress thread may have ended, ignore
                }

                if !show_progress {
                    info!("finished parsing {}", file_path.as_str());
                }
            },
        );

        // Page processing complete - no need to update counters as they're updated in real-time

        page += 1;
    }

    if let Some(pb) = pb {
        let unit = if use_pagination { "pages" } else { "batches" };
        let final_message = format!("Completed {} {}", page - 1, unit);
        pb.finish_with_message(final_message);
    }

    if !show_progress {
        let unit = if use_pagination { "pages" } else { "batches" };
        info!("Completed processing across {} {}", page - 1, unit);
    }

    // Retry phase for failed items
    let failed_count = {
        match failed_items.lock() {
            Ok(failed) => failed.len(),
            Err(e) => {
                warn!("Failed to lock failed_items mutex: {}", e);
                0
            }
        }
    };

    if failed_count > 0 {
        if !show_progress {
            info!("Starting retry phase for {} failed items", failed_count);
        }

        // Process retries sequentially to avoid overwhelming servers
        let mut retry_queue = {
            match failed_items.lock() {
                Ok(failed) => failed.clone(),
                Err(e) => {
                    warn!("Failed to lock failed_items mutex for retry: {}", e);
                    vec![]
                }
            }
        };

        let mut retry_stats = HashMap::new();
        let mut total_retries = 0;
        let mut successful_retries = 0;

        while !retry_queue.is_empty() {
            let mut new_failures = Vec::new();

            for mut failed_item in retry_queue {
                if !failed_item.should_retry() {
                    // Max retries reached
                    *retry_stats.entry("max_retries_reached").or_insert(0) += 1;
                    continue;
                }

                let delay = failed_item.next_delay();
                if !show_progress {
                    info!(
                        "Retrying {} (attempt {}/{}) after {}s delay",
                        failed_item.item.url.as_str(),
                        failed_item.attempt_count + 1,
                        MAX_RETRIES,
                        delay.as_secs()
                    );
                }

                thread::sleep(delay);
                total_retries += 1;

                // Determine file path (cache or remote URL)
                let file_path = if let Some(ref cache_dir) = cache_dir {
                    let cache_path = match url_to_cache_path(
                        cache_dir,
                        &failed_item.item.collector_id,
                        &failed_item.item.url,
                    ) {
                        Some(p) => p,
                        None => {
                            let error_msg = format!(
                                "Failed to construct cache path for {}",
                                failed_item.item.url
                            );
                            failed_item.increment_attempt(error_msg);
                            new_failures.push(failed_item);
                            continue;
                        }
                    };

                    // Delete any existing cached file (might be corrupted) and re-download
                    let _ = std::fs::remove_file(&cache_path);
                    if let Err(e) = download_to_cache(&failed_item.item.url, &cache_path) {
                        let error_msg = format!(
                            "Retry failed to download {} to cache: {}",
                            failed_item.item.url, e
                        );
                        if !show_progress {
                            warn!("{}", error_msg);
                        }
                        failed_item.increment_attempt(error_msg);
                        new_failures.push(failed_item);
                        continue;
                    }
                    cache_path.to_string_lossy().to_string()
                } else {
                    failed_item.item.url.clone()
                };

                let parser = match filters.to_parser(file_path.as_str()) {
                    Ok(p) => p,
                    Err(e) => {
                        let error_msg = format!("Retry failed to parse {}: {}", file_path, e);
                        if !show_progress {
                            warn!("{}", error_msg);
                        }
                        // Delete cached file on parse failure
                        if cache_dir.is_some() {
                            let _ = std::fs::remove_file(&file_path);
                        }
                        failed_item.increment_attempt(error_msg);
                        new_failures.push(failed_item);
                        continue;
                    }
                };

                let mut elems_count = 0;
                let mut parse_successful = true;

                for elem in parser {
                    if sender
                        .send(WriterMessage::Element(
                            Box::new(elem),
                            failed_item.item.collector_id.clone(),
                        ))
                        .is_err()
                    {
                        // Channel closed, mark as failed
                        parse_successful = false;
                        break;
                    }
                    elems_count += 1;
                }

                // Send file completion signal for retry as well
                if parse_successful && sender.send(WriterMessage::FileComplete).is_err() {
                    parse_successful = false;
                }

                if parse_successful {
                    successful_retries += 1;
                    // Retry successful - progress already tracked by main processing
                    if !show_progress {
                        info!(
                            "Successfully retried {} (found {} messages)",
                            failed_item.item.url.as_str(),
                            elems_count
                        );
                    }
                } else {
                    let error_msg = "Retry failed: channel closed during processing".to_string();
                    failed_item.increment_attempt(error_msg);
                    new_failures.push(failed_item);
                }
            }

            retry_queue = new_failures;
        }

        // Log retry statistics
        let final_failures = retry_queue.len();
        if !show_progress {
            info!(
                "Retry phase completed: {} total retry attempts, {} successful, {} final failures",
                total_retries, successful_retries, final_failures
            );

            if final_failures > 0 {
                warn!(
                    "Warning: {} files could not be processed after {} retry attempts",
                    final_failures, MAX_RETRIES
                );
            }
        }
    }

    // Close channels to signal completion
    drop(sender);
    drop(progress_sender);

    // wait for the output thread to stop
    if let Err(e) = writer_thread.join() {
        eprintln!("Writer thread failed: {:?}", e);
    }

    if let Err(e) = progress_thread.join() {
        eprintln!("Progress thread failed: {:?}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_url_to_cache_path_ripe_ris() {
        let cache_dir = PathBuf::from("/cache");
        let url = "https://data.ris.ripe.net/rrc00/2024.01/updates.20240101.0000.gz";
        let collector = "rrc00";

        let result = url_to_cache_path(&cache_dir, collector, url);
        assert_eq!(
            result,
            Some(PathBuf::from(
                "/cache/rrc00/2024.01/updates.20240101.0000.gz"
            ))
        );
    }

    #[test]
    fn test_url_to_cache_path_routeviews_main() {
        let cache_dir = PathBuf::from("/cache");
        // route-views2 uses /bgpdata/ path (collector not in URL path)
        let url = "http://archive.routeviews.org/bgpdata/2024.01/UPDATES/updates.20240101.0000.bz2";
        let collector = "route-views2";

        let result = url_to_cache_path(&cache_dir, collector, url);
        assert_eq!(
            result,
            Some(PathBuf::from(
                "/cache/route-views2/bgpdata/2024.01/UPDATES/updates.20240101.0000.bz2"
            ))
        );
    }

    #[test]
    fn test_url_to_cache_path_routeviews_named() {
        let cache_dir = PathBuf::from("/cache");
        // route-views6 has collector in URL path
        let url = "http://archive.routeviews.org/route-views6/bgpdata/2024.01/UPDATES/updates.bz2";
        let collector = "route-views6";

        let result = url_to_cache_path(&cache_dir, collector, url);
        assert_eq!(
            result,
            Some(PathBuf::from(
                "/cache/route-views6/bgpdata/2024.01/UPDATES/updates.bz2"
            ))
        );
    }

    #[test]
    fn test_url_to_cache_path_invalid_url() {
        let cache_dir = PathBuf::from("/cache");
        let url = "not-a-valid-url";
        let collector = "rrc00";

        let result = url_to_cache_path(&cache_dir, collector, url);
        assert_eq!(result, None);
    }

    #[test]
    fn test_url_to_cache_path_ftp_url() {
        let cache_dir = PathBuf::from("/cache");
        // FTP URLs are not HTTP/HTTPS, should return None
        let url = "ftp://example.com/data/file.gz";
        let collector = "test";

        let result = url_to_cache_path(&cache_dir, collector, url);
        assert_eq!(result, None);
    }
}
