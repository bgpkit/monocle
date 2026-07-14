//! Search lens module
//!
//! This module provides filter types for searching BGP messages across multiple MRT files.
//! The filter types can optionally derive Clap's Args trait when the `cli` feature is enabled.
//!
//! # Progress Tracking
//!
//! The `SearchLens` supports progress tracking through callbacks. This is useful for
//! building GUI applications or showing progress in CLI tools.
//!
//! ```rust,ignore
//! use monocle::lens::search::{SearchLens, SearchFilters, SearchProgress};
//! use std::sync::Arc;
//!
//! let lens = SearchLens::new();
//! let filters = SearchFilters { /* ... */ };
//!
//! let callback = Arc::new(|progress: SearchProgress| {
//!     match progress {
//!         SearchProgress::FilesFound { count } => {
//!             println!("Found {} files to process", count);
//!         }
//!         SearchProgress::FileCompleted { file_index, total_files, .. } => {
//!             let pct = (file_index + 1) as f64 / total_files as f64 * 100.0;
//!             println!("Progress: {:.1}%", pct);
//!         }
//!         _ => {}
//!     }
//! });
//!
//! // Search with progress tracking
//! lens.search_with_progress(&filters, Some(callback), |elem, collector| {
//!     // Handle each element
//! })?;
//! ```

mod query_builder;

pub use query_builder::{build_prefix_filter, SearchFilterSpec, SearchQueryBuilder};

use crate::lens::parse::ParseFilters;
use anyhow::Result;
use bgpkit_broker::BrokerItem;
use bgpkit_parser::BgpElem;
use bgpkit_parser::BgpkitParser;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(feature = "cli")]
use clap::{Args, ValueEnum};

// =============================================================================
// Progress Tracking Types
// =============================================================================

/// Progress information for search operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SearchProgress {
    /// Querying the broker for available files
    QueryingBroker,

    /// Broker query complete, files found
    FilesFound {
        /// Number of files to process
        count: usize,
    },

    /// Started processing a file
    FileStarted {
        /// Index of the file (0-based)
        file_index: usize,
        /// Total number of files
        total_files: usize,
        /// URL of the file being processed
        file_url: String,
        /// Collector ID
        collector: String,
    },

    /// Completed processing a file
    FileCompleted {
        /// Index of the file (0-based)
        file_index: usize,
        /// Total number of files
        total_files: usize,
        /// Number of messages found in this file
        messages_found: u64,
        /// Whether the file was processed successfully
        success: bool,
        /// Error message if failed
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Overall progress update (can be used for percentage display)
    ProgressUpdate {
        /// Number of files completed
        files_completed: usize,
        /// Total number of files
        total_files: usize,
        /// Total messages found so far
        total_messages: u64,
        /// Percentage complete (0.0 - 100.0)
        percent_complete: f64,
        /// Elapsed time in seconds
        elapsed_secs: f64,
        /// Estimated time remaining in seconds (if available)
        #[serde(skip_serializing_if = "Option::is_none")]
        eta_secs: Option<f64>,
    },

    /// All processing completed
    Completed {
        /// Total number of files processed
        total_files: usize,
        /// Number of successful files
        successful_files: usize,
        /// Number of failed files
        failed_files: usize,
        /// Total messages found
        total_messages: u64,
        /// Total duration in seconds
        duration_secs: f64,
        /// Average processing rate in files per second
        #[serde(skip_serializing_if = "Option::is_none")]
        files_per_sec: Option<f64>,
    },
}

/// Type alias for search progress callback function
///
/// The callback receives `SearchProgress` updates and can be used to
/// update UI elements, log progress, or perform other actions.
///
/// Note: This callback may be called from multiple threads concurrently
/// when processing files in parallel.
pub type SearchProgressCallback = Arc<dyn Fn(SearchProgress) + Send + Sync>;

/// Type alias for element handler function
///
/// Called for each BGP element found during search, along with the collector ID.
pub type ElementHandler = Arc<dyn Fn(BgpElem, String) + Send + Sync>;

/// Default number of elements per batch when no explicit `batch_size` is set.
const DEFAULT_SEARCH_BATCH_SIZE: usize = 64;

/// Runtime options for executing a search.
#[derive(Clone)]
pub struct SearchExecutionOptions {
    /// Number of rayon worker threads for this search. `None` or `Some(0)` uses rayon's default.
    pub concurrency: Option<usize>,
    /// Reusable rayon thread pool supplied by the caller. Takes precedence over `concurrency`.
    pub thread_pool: Option<Arc<rayon::ThreadPool>>,
    /// Maximum number of elements to emit. `None` means unlimited.
    pub max_results: Option<u64>,
    /// Maximum wall-clock runtime after broker query starts. `None` means no timeout.
    pub timeout: Option<Duration>,
    /// Optional external cancellation flag.
    pub cancel_flag: Option<Arc<AtomicBool>>,
    /// Number of elements per batch passed to [`SearchSink::on_elements`].
    pub batch_size: usize,
}

impl Default for SearchExecutionOptions {
    fn default() -> Self {
        Self {
            concurrency: None,
            thread_pool: None,
            max_results: None,
            timeout: None,
            cancel_flag: None,
            batch_size: DEFAULT_SEARCH_BATCH_SIZE,
        }
    }
}

/// Reason a search execution stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchExitReason {
    Completed,
    Cancelled,
    Timeout,
    MaxResultsReached,
}

/// Search execution result including both summary and stop reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOutcome {
    pub summary: SearchSummary,
    pub exit_reason: SearchExitReason,
}

/// A batch of matched elements from a single MRT file.
pub struct SearchElementBatch {
    pub file_index: usize,
    pub file_url: String,
    pub collector: String,
    pub elements: Vec<BgpElem>,
}

/// Control returned by a search sink after receiving elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchControl {
    Continue,
    /// Stop the search early. The shared executor reports this as
    /// [`SearchExitReason::Cancelled`] because the sink is the active consumer
    /// of search results and no longer wants more data.
    Stop,
}

/// Sink used by the shared search executor.
pub trait SearchSink: Send + Sync {
    fn on_progress(&self, _progress: SearchProgress) {}

    fn on_elements(&self, batch: SearchElementBatch) -> SearchControl;
}

// =============================================================================
// Types
// =============================================================================

/// Dump type for BGP data
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(ValueEnum))]
pub enum SearchDumpType {
    /// BGP updates only
    #[default]
    Updates,
    /// BGP RIB dump only
    Rib,
    /// BGP RIB dump and BGP updates
    RibUpdates,
}

// =============================================================================
// Args
// =============================================================================

/// Filters for searching BGP messages across multiple MRT files
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(Args))]
pub struct SearchFilters {
    #[cfg_attr(feature = "cli", clap(flatten))]
    #[serde(flatten)]
    pub parse_filters: ParseFilters,

    /// Filter by collector, e.g., rrc00 or route-views2
    #[cfg_attr(feature = "cli", clap(short = 'c', long))]
    pub collector: Option<String>,

    /// Filter by route collection project, i.e., riperis or routeviews
    #[cfg_attr(feature = "cli", clap(short = 'P', long))]
    pub project: Option<String>,

    /// Specify data dump type to search (updates or RIB dump)
    #[cfg_attr(feature = "cli", clap(short = 'D', long, default_value_t, value_enum))]
    #[serde(default)]
    pub dump_type: SearchDumpType,
}

impl SearchFilters {
    /// Query broker items based on filters
    pub fn to_broker_items(&self) -> Result<Vec<BrokerItem>> {
        self.build_broker()?
            .query()
            .map_err(|_| anyhow::anyhow!("broker query error: please check filters are valid"))
    }

    /// Build a broker from the filters
    pub fn build_broker(&self) -> Result<bgpkit_broker::BgpkitBroker> {
        let (ts_start, ts_end) = self.parse_filters.parse_start_end_strings()?;

        let mut broker = bgpkit_broker::BgpkitBroker::new()
            .ts_start(ts_start)
            .ts_end(ts_end)
            .page_size(1000);

        if let Some(project) = &self.project {
            broker = broker.project(project.as_str());
        }
        if let Some(collector) = &self.collector {
            broker = broker.collector_id(collector.as_str());
        }

        match self.dump_type {
            SearchDumpType::Updates => {
                broker = broker.data_type("updates");
            }
            SearchDumpType::Rib => {
                broker = broker.data_type("rib");
            }
            SearchDumpType::RibUpdates => {
                // do nothing here -> getting all RIB and updates
            }
        }

        Ok(broker)
    }

    /// Validate the filters
    pub fn validate(&self) -> Result<()> {
        let _ = self.parse_filters.parse_start_end_strings()?;
        Ok(())
    }

    /// Convert filters to a BgpkitParser for a given file.
    ///
    /// RIB dumps are snapshots: their per-route timestamps describe when a route
    /// was learned, not the timestamp of the dump. Applying a search time window
    /// to those timestamps would drop stable routes from a matching RIB file.
    pub fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        self.parser_filters().to_parser(file_path)
    }

    fn parser_filters(&self) -> ParseFilters {
        let mut parse_filters = self.parse_filters.clone();
        if self.dump_type == SearchDumpType::Rib {
            parse_filters.start_ts = None;
            parse_filters.end_ts = None;
            parse_filters.duration = None;
        }
        parse_filters
    }
}

// =============================================================================
// Search Result Types
// =============================================================================

/// Summary of search results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSummary {
    /// Total number of files processed
    pub total_files: usize,
    /// Number of successful files
    pub successful_files: usize,
    /// Number of failed files
    pub failed_files: usize,
    /// Total messages found
    pub total_messages: u64,
    /// Total duration in seconds
    pub duration_secs: f64,
}

// =============================================================================
// Lens
// =============================================================================

/// Search lens for BGP message search operations
///
/// This lens provides high-level operations for searching BGP messages
/// across multiple MRT files using the BGPKIT broker, with optional
/// progress tracking support.
///
/// # Example
///
/// ```rust,ignore
/// use monocle::lens::search::{SearchLens, SearchFilters, SearchProgress};
/// use std::sync::Arc;
///
/// let lens = SearchLens::new();
/// let filters = SearchFilters { /* ... */ };
///
/// // Simple search without progress tracking
/// let items = lens.query_broker(&filters)?;
/// for item in items {
///     let parser = lens.create_parser(&filters, &item.url)?;
///     for elem in parser {
///         println!("{}", elem);
///     }
/// }
///
/// // Search with progress tracking
/// let callback = Arc::new(|progress: SearchProgress| {
///     if let SearchProgress::ProgressUpdate { percent_complete, .. } = progress {
///         println!("Progress: {:.1}%", percent_complete);
///     }
/// });
///
/// let handler = Arc::new(|elem: BgpElem, collector: String| {
///     println!("{} from {}", elem, collector);
/// });
///
/// let summary = lens.search_with_progress(&filters, Some(callback), handler)?;
/// println!("Found {} messages", summary.total_messages);
/// ```
pub struct SearchLens;

impl SearchLens {
    /// Create a new search lens
    pub fn new() -> Self {
        Self
    }

    /// Query broker items based on filters
    pub fn query_broker(&self, filters: &SearchFilters) -> Result<Vec<BrokerItem>> {
        filters.to_broker_items()
    }

    /// Build a broker from filters
    pub fn build_broker(&self, filters: &SearchFilters) -> Result<bgpkit_broker::BgpkitBroker> {
        filters.build_broker()
    }

    /// Create a parser for a specific file
    pub fn create_parser(
        &self,
        filters: &SearchFilters,
        file_path: &str,
    ) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        filters.to_parser(file_path)
    }

    /// Validate filters
    pub fn validate_filters(&self, filters: &SearchFilters) -> Result<()> {
        filters.validate()
    }

    /// Search BGP messages with progress tracking
    ///
    /// This method queries the broker, processes all matching files in parallel,
    /// and reports progress through the callback.
    ///
    /// # Arguments
    ///
    /// * `filters` - Filters to apply during search
    /// * `progress_callback` - Optional callback to receive progress updates
    /// * `element_handler` - Handler called for each found BGP element
    ///
    /// # Returns
    ///
    /// A summary of the search results
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use monocle::lens::search::{SearchLens, SearchFilters, SearchProgress};
    /// use std::sync::Arc;
    ///
    /// let lens = SearchLens::new();
    /// let filters = SearchFilters { /* ... */ };
    ///
    /// let callback = Arc::new(|progress: SearchProgress| {
    ///     match progress {
    ///         SearchProgress::FilesFound { count } => {
    ///             println!("Found {} files to process", count);
    ///         }
    ///         SearchProgress::ProgressUpdate { percent_complete, total_messages, .. } => {
    ///             println!("Progress: {:.1}%, found {} messages", percent_complete, total_messages);
    ///         }
    ///         SearchProgress::Completed { total_messages, duration_secs, .. } => {
    ///             println!("Done: {} messages in {:.2}s", total_messages, duration_secs);
    ///         }
    ///         _ => {}
    ///     }
    /// });
    ///
    /// let handler = Arc::new(|elem: BgpElem, collector: String| {
    ///     // Process element
    /// });
    ///
    /// let summary = lens.search_with_progress(&filters, Some(callback), handler)?;
    /// ```
    pub fn search_with_progress(
        &self,
        filters: &SearchFilters,
        progress_callback: Option<SearchProgressCallback>,
        element_handler: ElementHandler,
    ) -> Result<SearchSummary> {
        struct CallbackSink {
            progress_callback: Option<SearchProgressCallback>,
            element_handler: ElementHandler,
        }

        impl SearchSink for CallbackSink {
            fn on_progress(&self, progress: SearchProgress) {
                if let Some(ref cb) = self.progress_callback {
                    cb(progress);
                }
            }

            fn on_elements(&self, batch: SearchElementBatch) -> SearchControl {
                for elem in batch.elements {
                    (self.element_handler)(elem, batch.collector.clone());
                }
                SearchControl::Continue
            }
        }

        let sink = Arc::new(CallbackSink {
            progress_callback,
            element_handler,
        });
        let outcome = self.search_with_options(filters, SearchExecutionOptions::default(), sink)?;
        Ok(outcome.summary)
    }

    /// Search BGP messages using the shared executor.
    pub fn search_with_options(
        &self,
        filters: &SearchFilters,
        options: SearchExecutionOptions,
        sink: Arc<dyn SearchSink>,
    ) -> Result<SearchOutcome> {
        let start_time = Instant::now();
        let deadline = options.timeout.map(|timeout| start_time + timeout);

        sink.on_progress(SearchProgress::QueryingBroker);

        let items = self.query_broker(filters)?;
        let total_files = items.len();
        sink.on_progress(SearchProgress::FilesFound { count: total_files });

        if deadline.is_some_and(|dl| Instant::now() >= dl) {
            return Ok(SearchOutcome {
                summary: SearchSummary {
                    total_files,
                    successful_files: 0,
                    failed_files: 0,
                    total_messages: 0,
                    duration_secs: start_time.elapsed().as_secs_f64(),
                },
                exit_reason: SearchExitReason::Timeout,
            });
        }

        if total_files == 0 {
            let duration_secs = start_time.elapsed().as_secs_f64();
            let summary = SearchSummary {
                total_files: 0,
                successful_files: 0,
                failed_files: 0,
                total_messages: 0,
                duration_secs,
            };
            sink.on_progress(SearchProgress::Completed {
                total_files: 0,
                successful_files: 0,
                failed_files: 0,
                total_messages: 0,
                duration_secs,
                files_per_sec: None,
            });
            return Ok(SearchOutcome {
                summary,
                exit_reason: SearchExitReason::Completed,
            });
        }

        let batch_size = options.batch_size.max(1);
        let max_results = options.max_results.filter(|max| *max > 0);
        let external_cancel = options.cancel_flag.clone();
        let stop_flag = AtomicBool::new(false);
        let exit_reason = AtomicU64::new(0);
        let files_completed = AtomicU64::new(0);
        let successful_files = AtomicU64::new(0);
        let failed_files = AtomicU64::new(0);
        let total_messages = AtomicU64::new(0);

        let run = || {
            items.into_par_iter().enumerate().for_each(|(index, item)| {
                let state = SearchWorkerState {
                    total_files,
                    start_time,
                    deadline,
                    batch_size,
                    max_results,
                    external_cancel: external_cancel.as_deref(),
                    stop_flag: &stop_flag,
                    exit_reason: &exit_reason,
                    files_completed: &files_completed,
                    successful_files: &successful_files,
                    failed_files: &failed_files,
                    total_messages: &total_messages,
                    sink: sink.as_ref(),
                };
                process_search_item(filters, index, item, state);
            });
        };

        if let Some(pool) = options.thread_pool.as_ref() {
            pool.install(run);
        } else {
            match options.concurrency {
                Some(n) if n > 0 => {
                    let pool = rayon::ThreadPoolBuilder::new().num_threads(n).build()?;
                    pool.install(run);
                }
                _ => run(),
            }
        }

        let duration_secs = start_time.elapsed().as_secs_f64();
        let final_successful = successful_files.load(Ordering::Relaxed) as usize;
        let final_failed = failed_files.load(Ordering::Relaxed) as usize;
        let final_messages = total_messages.load(Ordering::Relaxed);
        let files_per_sec = if duration_secs > 0.0 {
            Some(total_files as f64 / duration_secs)
        } else {
            None
        };

        let exit_reason = match exit_reason.load(Ordering::Relaxed) {
            1 => SearchExitReason::Cancelled,
            2 => SearchExitReason::Timeout,
            3 => SearchExitReason::MaxResultsReached,
            _ => SearchExitReason::Completed,
        };

        if matches!(
            exit_reason,
            SearchExitReason::Completed | SearchExitReason::MaxResultsReached
        ) {
            sink.on_progress(SearchProgress::Completed {
                total_files,
                successful_files: final_successful,
                failed_files: final_failed,
                total_messages: final_messages,
                duration_secs,
                files_per_sec,
            });
        }

        let summary = SearchSummary {
            total_files,
            successful_files: final_successful,
            failed_files: final_failed,
            total_messages: final_messages,
            duration_secs,
        };

        Ok(SearchOutcome {
            summary,
            exit_reason,
        })
    }

    /// Search and collect all BGP elements with progress tracking
    ///
    /// This is a convenience method that collects all elements into a Vec.
    /// For large searches, consider using `search_with_progress` with a custom
    /// handler to avoid high memory usage.
    ///
    /// # Arguments
    ///
    /// * `filters` - Filters to apply during search
    /// * `progress_callback` - Optional callback to receive progress updates
    ///
    /// # Returns
    ///
    /// A tuple of (elements, summary) where elements is a Vec of (BgpElem, collector_id) tuples
    pub fn search_and_collect(
        &self,
        filters: &SearchFilters,
        progress_callback: Option<SearchProgressCallback>,
    ) -> Result<(Vec<(BgpElem, String)>, SearchSummary)> {
        use std::sync::Mutex;

        let elements: Arc<Mutex<Vec<(BgpElem, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let elements_clone = Arc::clone(&elements);

        let handler: ElementHandler = Arc::new(move |elem, collector| {
            if let Ok(mut vec) = elements_clone.lock() {
                vec.push((elem, collector));
            }
        });

        let summary = self.search_with_progress(filters, progress_callback, handler)?;

        let result = Arc::try_unwrap(elements)
            .map_err(|_| anyhow::anyhow!("Failed to unwrap elements Arc"))?
            .into_inner()
            .map_err(|e| anyhow::anyhow!("Failed to get elements from Mutex: {}", e))?;

        Ok((result, summary))
    }
}

struct SearchWorkerState<'a> {
    total_files: usize,
    start_time: Instant,
    deadline: Option<Instant>,
    batch_size: usize,
    max_results: Option<u64>,
    external_cancel: Option<&'a AtomicBool>,
    stop_flag: &'a AtomicBool,
    exit_reason: &'a AtomicU64,
    files_completed: &'a AtomicU64,
    successful_files: &'a AtomicU64,
    failed_files: &'a AtomicU64,
    total_messages: &'a AtomicU64,
    sink: &'a dyn SearchSink,
}

fn process_search_item(
    filters: &SearchFilters,
    index: usize,
    item: BrokerItem,
    state: SearchWorkerState<'_>,
) {
    if search_should_stop(&state) {
        return;
    }

    let url = item.url.clone();
    let collector = item.collector_id.clone();

    state.sink.on_progress(SearchProgress::FileStarted {
        file_index: index,
        total_files: state.total_files,
        file_url: url.clone(),
        collector: collector.clone(),
    });

    let parser = match filters.to_parser(url.as_str()) {
        Ok(parser) => parser,
        Err(e) => {
            state.failed_files.fetch_add(1, Ordering::Relaxed);
            let completed = state.files_completed.fetch_add(1, Ordering::Relaxed) + 1;
            state.sink.on_progress(SearchProgress::FileCompleted {
                file_index: index,
                total_files: state.total_files,
                messages_found: 0,
                success: false,
                error: Some(e.to_string()),
            });
            send_progress_update(&state, completed);
            return;
        }
    };

    let mut file_messages = 0_u64;
    let mut batch = Vec::with_capacity(state.batch_size);

    for elem in parser {
        if search_should_stop(&state) {
            break;
        }

        batch.push(elem);
        if should_emit_batch(&state, batch.len()) {
            let accepted = emit_search_batch(&state, index, &url, &collector, &mut batch);
            file_messages += accepted;
        }
    }

    if !batch.is_empty() && !search_should_stop(&state) {
        let accepted = emit_search_batch(&state, index, &url, &collector, &mut batch);
        file_messages += accepted;
    }

    let stopped_reason = current_exit_reason(&state);
    let stopped_before_success = matches!(
        stopped_reason,
        Some(SearchExitReason::Cancelled | SearchExitReason::Timeout)
    );

    if stopped_before_success {
        state.failed_files.fetch_add(1, Ordering::Relaxed);
    } else {
        state.successful_files.fetch_add(1, Ordering::Relaxed);
    }

    let completed = state.files_completed.fetch_add(1, Ordering::Relaxed) + 1;
    state.sink.on_progress(SearchProgress::FileCompleted {
        file_index: index,
        total_files: state.total_files,
        messages_found: file_messages,
        success: !stopped_before_success,
        error: stopped_reason.and_then(|reason| match reason {
            SearchExitReason::Cancelled => Some("search cancelled".to_string()),
            SearchExitReason::Timeout => Some("search timed out".to_string()),
            _ => None,
        }),
    });
    send_progress_update(&state, completed);
}

fn search_should_stop(state: &SearchWorkerState<'_>) -> bool {
    if state.stop_flag.load(Ordering::Relaxed) {
        return true;
    }

    if let Some(cancel) = state.external_cancel {
        if cancel.load(Ordering::Relaxed) {
            state
                .exit_reason
                .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
                .ok();
            state.stop_flag.store(true, Ordering::Relaxed);
            return true;
        }
    }

    if let Some(deadline) = state.deadline {
        if Instant::now() >= deadline {
            state
                .exit_reason
                .compare_exchange(0, 2, Ordering::Relaxed, Ordering::Relaxed)
                .ok();
            state.stop_flag.store(true, Ordering::Relaxed);
            return true;
        }
    }

    false
}

fn should_emit_batch(state: &SearchWorkerState<'_>, batch_len: usize) -> bool {
    if batch_len >= state.batch_size {
        return true;
    }

    if let Some(max) = state.max_results {
        let emitted = state.total_messages.load(Ordering::Relaxed);
        let remaining = max.saturating_sub(emitted);
        return remaining == 0 || batch_len as u64 >= remaining;
    }

    false
}

fn emit_search_batch(
    state: &SearchWorkerState<'_>,
    file_index: usize,
    file_url: &str,
    collector: &str,
    batch: &mut Vec<BgpElem>,
) -> u64 {
    let original_len = batch.len();
    let accepted = reserve_result_slots(state, original_len as u64);
    if accepted == 0 {
        batch.clear();
        return 0;
    }

    if accepted < original_len as u64 {
        batch.truncate(accepted as usize);
    }

    let mut elements = Vec::with_capacity(state.batch_size);
    std::mem::swap(&mut elements, batch);
    let control = state.sink.on_elements(SearchElementBatch {
        file_index,
        file_url: file_url.to_string(),
        collector: collector.to_string(),
        elements,
    });

    if control == SearchControl::Stop {
        state
            .exit_reason
            .compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed)
            .ok();
        state.stop_flag.store(true, Ordering::Relaxed);
    }

    accepted
}

fn current_exit_reason(state: &SearchWorkerState<'_>) -> Option<SearchExitReason> {
    match state.exit_reason.load(Ordering::Relaxed) {
        1 => Some(SearchExitReason::Cancelled),
        2 => Some(SearchExitReason::Timeout),
        3 => Some(SearchExitReason::MaxResultsReached),
        _ => None,
    }
}

fn reserve_result_slots(state: &SearchWorkerState<'_>, wanted: u64) -> u64 {
    match state.max_results {
        None => {
            state.total_messages.fetch_add(wanted, Ordering::Relaxed);
            wanted
        }
        Some(max) => loop {
            let current = state.total_messages.load(Ordering::Relaxed);
            if current >= max {
                state
                    .exit_reason
                    .compare_exchange(0, 3, Ordering::Relaxed, Ordering::Relaxed)
                    .ok();
                state.stop_flag.store(true, Ordering::Relaxed);
                return 0;
            }

            let allowed = wanted.min(max - current);
            if state
                .total_messages
                .compare_exchange(
                    current,
                    current + allowed,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                if allowed < wanted || current + allowed >= max {
                    state
                        .exit_reason
                        .compare_exchange(0, 3, Ordering::Relaxed, Ordering::Relaxed)
                        .ok();
                    state.stop_flag.store(true, Ordering::Relaxed);
                }
                return allowed;
            }
        },
    }
}

fn send_progress_update(state: &SearchWorkerState<'_>, completed: u64) {
    let elapsed = state.start_time.elapsed().as_secs_f64();
    let percent = completed as f64 / state.total_files as f64 * 100.0;
    let eta = if completed > 0 && percent < 100.0 {
        let rate = elapsed / completed as f64;
        Some(rate * (state.total_files as u64 - completed) as f64)
    } else {
        None
    };

    state.sink.on_progress(SearchProgress::ProgressUpdate {
        files_completed: completed as usize,
        total_files: state.total_files,
        total_messages: state.total_messages.load(Ordering::Relaxed),
        percent_complete: percent,
        elapsed_secs: elapsed,
        eta_secs: eta,
    });
}

impl Default for SearchLens {
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
    fn test_rib_parser_filters_do_not_apply_snapshot_time_window() {
        let filters = SearchFilters {
            parse_filters: ParseFilters {
                start_ts: Some("2026-06-01T00:00:00Z".to_string()),
                end_ts: Some("2026-06-01T00:00:00Z".to_string()),
                ..Default::default()
            },
            dump_type: SearchDumpType::Rib,
            ..Default::default()
        };

        let parser_filters = filters.parser_filters();
        assert_eq!(parser_filters.start_ts, None);
        assert_eq!(parser_filters.end_ts, None);
    }

    #[test]
    fn test_pagination_logic() {
        // Create a test filter with a short time range to get manageable results
        let search_filters = SearchFilters {
            parse_filters: ParseFilters {
                origin_asn: Vec::new(),
                prefix: Vec::new(),
                include_super: false,
                include_sub: false,
                peer_ip: Vec::new(),
                peer_asn: Vec::new(),
                communities: Vec::new(),
                elem_type: None,
                start_ts: Some("2022-01-01T00:00:00Z".to_string()),
                end_ts: Some("2022-01-01T01:00:00Z".to_string()), // 1 hour window
                duration: None,
                as_path: None,
            },
            collector: None,
            project: None,
            dump_type: SearchDumpType::Updates,
        };

        // Test broker creation
        let base_broker = search_filters
            .build_broker()
            .expect("Failed to build broker");

        // Test pagination with small page size for testing
        let test_broker = base_broker.clone().page_size(10); // Small page for testing

        let mut total_items = 0;
        let mut page = 1i64;
        let mut pages_processed = 0;

        // Test pagination loop similar to main implementation
        loop {
            let items = match test_broker.clone().page(page).query_single_page() {
                Ok(items) => items,
                Err(e) => {
                    println!("Failed to fetch page {}: {}", page, e);
                    break;
                }
            };

            if items.is_empty() {
                println!("Reached empty page {}, stopping", page);
                break;
            }

            total_items += items.len();
            pages_processed += 1;

            println!(
                "Page {}: {} items (total: {})",
                page,
                items.len(),
                total_items
            );

            // Verify items have timestamps
            if let Some(first_item) = items.first() {
                println!(
                    "  First item timestamp: {}",
                    first_item.ts_start.format("%Y-%m-%d %H:%M UTC")
                );
            }

            page += 1;

            // Safety check to prevent infinite loops in test
            if pages_processed >= 5 || items.len() < 10 {
                println!(
                    "Test complete: processed {} pages with {} total items",
                    pages_processed, total_items
                );
                break;
            }
        }

        // Verify we processed some data
        assert!(total_items > 0, "Should have found some items");
        assert!(
            pages_processed > 0,
            "Should have processed at least one page"
        );

        println!("Pagination test completed successfully");
    }

    #[test]
    fn test_build_broker_with_filters() {
        let search_filters = SearchFilters {
            parse_filters: ParseFilters {
                origin_asn: Vec::new(),
                prefix: Vec::new(),
                include_super: false,
                include_sub: false,
                peer_ip: Vec::new(),
                peer_asn: Vec::new(),
                communities: Vec::new(),
                elem_type: None,
                start_ts: Some("2022-01-01T00:00:00Z".to_string()),
                end_ts: Some("2022-01-01T01:00:00Z".to_string()),
                duration: None,
                as_path: None,
            },
            collector: Some("rrc00".to_string()),
            project: Some("riperis".to_string()),
            dump_type: SearchDumpType::Updates,
        };

        let broker = search_filters
            .build_broker()
            .expect("Failed to build broker");

        // Test that we can get at least one page
        let items = broker
            .page(1)
            .query_single_page()
            .expect("Failed to query first page");

        println!("First page with filters: {} items", items.len());

        // Verify all items match the collector filter if any items found
        if !items.is_empty() {
            for item in &items {
                assert_eq!(
                    item.collector_id, "rrc00",
                    "Item collector should match filter"
                );
            }
            println!("All items correctly filtered by collector");
        }
    }

    #[test]
    fn test_search_progress_serialization() {
        // Test that progress types can be serialized for GUI communication
        let progress = SearchProgress::FilesFound { count: 42 };
        let json = serde_json::to_string(&progress).expect("Failed to serialize");
        assert!(json.contains("42"));

        let progress = SearchProgress::ProgressUpdate {
            files_completed: 10,
            total_files: 100,
            total_messages: 5000,
            percent_complete: 10.0,
            elapsed_secs: 5.5,
            eta_secs: Some(49.5),
        };
        let json = serde_json::to_string(&progress).expect("Failed to serialize");
        assert!(json.contains("percent_complete"));
    }

    struct CountingSink {
        batches: AtomicU64,
        elements: AtomicU64,
    }

    impl CountingSink {
        fn new() -> Self {
            Self {
                batches: AtomicU64::new(0),
                elements: AtomicU64::new(0),
            }
        }
    }

    impl SearchSink for CountingSink {
        fn on_elements(&self, batch: SearchElementBatch) -> SearchControl {
            self.batches.fetch_add(1, Ordering::Relaxed);
            self.elements
                .fetch_add(batch.elements.len() as u64, Ordering::Relaxed);
            SearchControl::Continue
        }
    }

    fn with_worker_state<F>(
        max_results: Option<u64>,
        cancel_flag: Option<&AtomicBool>,
        deadline: Option<Instant>,
        sink: &dyn SearchSink,
        f: F,
    ) where
        F: FnOnce(SearchWorkerState<'_>),
    {
        let stop_flag = AtomicBool::new(false);
        let exit_reason = AtomicU64::new(0);
        let files_completed = AtomicU64::new(0);
        let successful_files = AtomicU64::new(0);
        let failed_files = AtomicU64::new(0);
        let total_messages = AtomicU64::new(0);

        f(SearchWorkerState {
            total_files: 1,
            start_time: Instant::now(),
            deadline,
            batch_size: 4,
            max_results,
            external_cancel: cancel_flag,
            stop_flag: &stop_flag,
            exit_reason: &exit_reason,
            files_completed: &files_completed,
            successful_files: &successful_files,
            failed_files: &failed_files,
            total_messages: &total_messages,
            sink,
        });
    }

    #[test]
    fn test_reserve_result_slots_truncates_at_max_results() {
        let sink = CountingSink::new();
        with_worker_state(Some(3), None, None, &sink, |state| {
            assert_eq!(reserve_result_slots(&state, 2), 2);
            assert_eq!(state.total_messages.load(Ordering::Relaxed), 2);
            assert!(should_emit_batch(&state, 1));
            assert_eq!(reserve_result_slots(&state, 2), 1);
            assert_eq!(state.total_messages.load(Ordering::Relaxed), 3);
            assert_eq!(
                current_exit_reason(&state),
                Some(SearchExitReason::MaxResultsReached)
            );
            assert!(state.stop_flag.load(Ordering::Relaxed));
        });
    }

    #[test]
    fn test_emit_search_batch_reuses_batch_capacity() {
        let sink = CountingSink::new();
        with_worker_state(Some(3), None, None, &sink, |state| {
            let mut batch = vec![
                BgpElem::default(),
                BgpElem::default(),
                BgpElem::default(),
                BgpElem::default(),
            ];
            let accepted = emit_search_batch(&state, 0, "file", "collector", &mut batch);
            assert_eq!(accepted, 3);
            assert_eq!(sink.batches.load(Ordering::Relaxed), 1);
            assert_eq!(sink.elements.load(Ordering::Relaxed), 3);
            assert!(batch.is_empty());
            assert!(batch.capacity() >= state.batch_size);
        });
    }

    #[test]
    fn test_search_should_stop_cancel_and_timeout() {
        let sink = CountingSink::new();
        let cancel = AtomicBool::new(true);
        with_worker_state(None, Some(&cancel), None, &sink, |state| {
            assert!(search_should_stop(&state));
            assert_eq!(
                current_exit_reason(&state),
                Some(SearchExitReason::Cancelled)
            );
        });

        with_worker_state(
            None,
            None,
            Some(Instant::now() - Duration::from_secs(1)),
            &sink,
            |state| {
                assert!(search_should_stop(&state));
                assert_eq!(current_exit_reason(&state), Some(SearchExitReason::Timeout));
            },
        );
    }

    #[test]
    fn test_search_filters_validate_with_communities() {
        let filters = SearchFilters {
            parse_filters: ParseFilters {
                communities: vec!["*:100".to_string(), "15169:*".to_string()],
                start_ts: Some("2025-01-01T00:00:00Z".to_string()),
                end_ts: Some("2025-01-01T01:00:00Z".to_string()),
                ..Default::default()
            },
            collector: Some("rrc00".to_string()),
            project: Some("riperis".to_string()),
            dump_type: SearchDumpType::Updates,
        };

        assert!(filters.validate().is_ok());
    }
}
