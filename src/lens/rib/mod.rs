//! RIB reconstruction lens.
//!
//! This module reconstructs final RIB state at arbitrary timestamps by:
//! 1. Selecting the latest RIB before each target time
//! 2. Replaying overlapping updates up to the exact target time
//! 3. Materializing only the final route state for each requested `rib_ts`

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use bgpkit_broker::{BgpkitBroker, BrokerItem};
use bgpkit_parser::models::ElemType;
use bgpkit_parser::BgpElem;
use chrono::{DateTime, Duration};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::config::MonocleConfig;
use crate::database::{
    MonocleDatabase, RibRouteKey, RibStateStore, StoredRibEntry, StoredRibUpdate,
};
use crate::lens::country::CountryLens;
use crate::lens::parse::ParseFilters;
use crate::lens::time::TimeLens;

#[cfg(feature = "cli")]
use clap::Args;

const FULL_FEED_V4_THRESHOLD: u32 = 800_000;
const FULL_FEED_V6_THRESHOLD: u32 = 100_000;
const RIB_LOOKBACK_HOURS: i64 = 24 * 30;

type FullFeedAllowlists = HashMap<String, HashSet<(String, u32)>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(Args))]
pub struct RibFilters {
    /// Filter by origin AS Number(s), comma-separated. Prefix with ! to exclude.
    #[cfg_attr(feature = "cli", clap(short = 'o', long, value_delimiter = ','))]
    #[serde(default)]
    pub origin_asn: Vec<String>,

    /// Filter by origin ASN registration country.
    #[cfg_attr(feature = "cli", clap(short = 'C', long))]
    pub country: Option<String>,

    /// Filter by network prefix(es), comma-separated. Prefix with ! to exclude.
    #[cfg_attr(feature = "cli", clap(short = 'p', long, value_delimiter = ','))]
    #[serde(default)]
    pub prefix: Vec<String>,

    /// Include super-prefixes when filtering.
    #[cfg_attr(feature = "cli", clap(short = 's', long))]
    #[serde(default)]
    pub include_super: bool,

    /// Include sub-prefixes when filtering.
    #[cfg_attr(feature = "cli", clap(short = 'S', long))]
    #[serde(default)]
    pub include_sub: bool,

    /// Filter by peer ASN(s), comma-separated. Prefix with ! to exclude.
    #[cfg_attr(feature = "cli", clap(short = 'J', long, value_delimiter = ','))]
    #[serde(default)]
    pub peer_asn: Vec<String>,

    /// Filter by AS path regex string.
    #[cfg_attr(feature = "cli", clap(short = 'a', long))]
    pub as_path: Option<String>,

    /// Filter by collector, e.g., rrc00 or route-views2.
    #[cfg_attr(feature = "cli", clap(short = 'c', long))]
    pub collector: Option<String>,

    /// Filter by route collection project, i.e. riperis or routeviews.
    #[cfg_attr(feature = "cli", clap(short = 'P', long))]
    pub project: Option<String>,

    /// Keep only full-feed peers based on broker peer metadata.
    #[cfg_attr(feature = "cli", clap(long))]
    #[serde(default)]
    pub full_feed_only: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "cli", derive(Args))]
pub struct RibArgs {
    /// Target RIB timestamp operand. Repeat to request multiple snapshots.
    #[cfg_attr(feature = "cli", clap(value_name = "RIB_TS", required = true))]
    #[serde(default)]
    pub rib_ts: Vec<String>,

    #[cfg_attr(feature = "cli", clap(flatten))]
    #[serde(flatten)]
    pub filters: RibFilters,

    /// SQLite output file path.
    #[cfg_attr(feature = "cli", clap(long))]
    pub sqlite_path: Option<PathBuf>,
}

impl RibArgs {
    pub fn normalized_rib_ts(&self) -> Result<Vec<i64>> {
        let time_lens = TimeLens::new();
        let mut timestamps = BTreeSet::new();

        for value in &self.rib_ts {
            let ts = time_lens
                .parse_time_string(value)
                .map_err(|e| anyhow!("Invalid RIB timestamp '{}': {}", value, e))?
                .timestamp();
            timestamps.insert(ts);
        }

        if timestamps.is_empty() {
            return Err(anyhow!("At least one RIB timestamp is required"));
        }

        Ok(timestamps.into_iter().collect())
    }

    pub fn validate(&self) -> Result<Vec<i64>> {
        let normalized_ts = self.normalized_rib_ts()?;

        let parse_filters = ParseFilters {
            origin_asn: self.filters.origin_asn.clone(),
            prefix: self.filters.prefix.clone(),
            include_super: self.filters.include_super,
            include_sub: self.filters.include_sub,
            peer_asn: self.filters.peer_asn.clone(),
            as_path: self.filters.as_path.clone(),
            ..Default::default()
        };
        parse_filters.validate()?;

        if let Some(as_path) = &self.filters.as_path {
            Regex::new(as_path)
                .map_err(|e| anyhow!("Invalid --as-path regex '{}': {}", as_path, e))?;
        }

        if normalized_ts.len() > 1 && self.sqlite_path.is_none() {
            return Err(anyhow!("Multiple RIB timestamps require --sqlite-path."));
        }

        Ok(normalized_ts)
    }
}

#[derive(Debug, Clone)]
pub struct RibRunSummary {
    pub rib_ts: Vec<i64>,
    pub collectors_processed: usize,
    pub groups_processed: usize,
}

#[derive(Debug, Clone)]
struct RibReplayGroup {
    collector: String,
    rib_item: BrokerItem,
    rib_ts: Vec<i64>,
    updates: Vec<BrokerItem>,
}

#[derive(Debug, Clone)]
enum DeltaOp {
    Upsert(StoredRibEntry),
    Delete(RibRouteKey),
}

#[derive(Debug, Clone)]
struct OriginFilter {
    values: HashSet<u32>,
    negated: bool,
}

pub struct RibLens<'a> {
    db: &'a MonocleDatabase,
    config: &'a MonocleConfig,
}

impl<'a> RibLens<'a> {
    pub fn new(db: &'a MonocleDatabase, config: &'a MonocleConfig) -> Self {
        Self { db, config }
    }

    /// Reconstruct RIB snapshots at specified timestamps.
    ///
    /// The `snapshot_visitor` callback is invoked for each snapshot with:
    /// - `i64`: The target RIB timestamp
    /// - `&RibStateStore`: The final reconstructed RIB state
    /// - `&[StoredRibUpdate]`: Filtered updates that contributed to this snapshot
    ///   (empty for the first/base RIB, populated for subsequent RIBs)
    pub fn reconstruct_snapshots<F>(
        &self,
        args: &RibArgs,
        no_update: bool,
        mut snapshot_visitor: F,
    ) -> Result<RibRunSummary>
    where
        F: FnMut(i64, &RibStateStore, &[StoredRibUpdate]) -> Result<()>,
    {
        let normalized_ts = args.validate()?;
        let country_asns = self.resolve_country_asns(args.filters.country.as_deref(), no_update)?;
        let origin_filter = Self::parse_origin_filter(&args.filters.origin_asn)?;
        let as_path_regex = Self::compile_as_path_regex(args.filters.as_path.as_deref())?;
        let groups = self.resolve_replay_groups(args, &normalized_ts)?;

        let allowlists = if args.filters.full_feed_only {
            self.build_full_feed_allowlists(&groups)?
        } else {
            HashMap::new()
        };

        for group in &groups {
            let mut state_store = RibStateStore::new_temp()?;
            // Base RIB files contain the full routing table snapshot at dump time.
            // Individual entry timestamps reflect when each route was learned, which
            // can be much earlier than the dump time.  We must NOT apply a start_ts
            // filter here — doing so would silently drop valid entries whose learned-
            // time predates the RIB dump timestamp (the common case for stable routes).
            let safe_base_filters = self.safe_rib_filters(args);

            info!(
                "Loading base RIB for {}: {}",
                group.collector, group.rib_item.url
            );
            let load_start = std::time::Instant::now();

            self.load_base_rib(
                &mut state_store,
                &group.collector,
                &group.rib_item,
                &safe_base_filters,
                country_asns.as_ref(),
                origin_filter.as_ref(),
                as_path_regex.as_ref(),
                allowlists.get(group.collector.as_str()),
            )?;

            info!(
                "Base RIB loaded: {} entries in {:.1}s",
                state_store.count()?,
                load_start.elapsed().as_secs_f64()
            );

            // If the first target timestamp equals the RIB time, emit it immediately
            // with empty updates (it's the base RIB, not built from updates)
            let rib_ts = group.rib_item.ts_start.and_utc().timestamp();
            if group
                .rib_ts
                .first()
                .map(|&ts| ts == rib_ts)
                .unwrap_or(false)
            {
                snapshot_visitor(group.rib_ts[0], &state_store, &[])?;
                // Create a new group with remaining timestamps for replay
                let remaining_ts: Vec<i64> = group.rib_ts.iter().skip(1).copied().collect();
                if !remaining_ts.is_empty() {
                    let mut new_group = group.clone();
                    new_group.rib_ts = remaining_ts;
                    self.replay_updates(
                        &mut state_store,
                        &new_group,
                        args,
                        country_asns.as_ref(),
                        origin_filter.as_ref(),
                        as_path_regex.as_ref(),
                        allowlists.get(group.collector.as_str()),
                        &mut snapshot_visitor,
                    )?;
                }
            } else {
                self.replay_updates(
                    &mut state_store,
                    group,
                    args,
                    country_asns.as_ref(),
                    origin_filter.as_ref(),
                    as_path_regex.as_ref(),
                    allowlists.get(group.collector.as_str()),
                    &mut snapshot_visitor,
                )?;
            }
        }

        let collector_count = groups
            .iter()
            .map(|group| group.collector.as_str())
            .collect::<HashSet<_>>()
            .len();

        Ok(RibRunSummary {
            rib_ts: normalized_ts,
            collectors_processed: collector_count,
            groups_processed: groups.len(),
        })
    }

    pub fn file_name_prefix(&self, args: &RibArgs, rib_ts: &[i64]) -> Result<String> {
        let base = if rib_ts.len() == 1 {
            format!(
                "monocle-rib-{}",
                Self::format_rib_ts_for_filename(rib_ts[0])?
            )
        } else {
            format!(
                "monocle-rib-{}-{}",
                Self::format_rib_ts_for_filename(
                    *rib_ts
                        .first()
                        .ok_or_else(|| anyhow!("missing first rib_ts"))?
                )?,
                Self::format_rib_ts_for_filename(
                    *rib_ts
                        .last()
                        .ok_or_else(|| anyhow!("missing last rib_ts"))?
                )?,
            )
        };

        let slug = self.filter_slug(&args.filters)?;
        if slug.is_empty() {
            Ok(base)
        } else {
            Ok(format!("{}-{}", base, slug))
        }
    }

    fn resolve_country_asns(
        &self,
        country: Option<&str>,
        no_update: bool,
    ) -> Result<Option<HashSet<u32>>> {
        let Some(country) = country else {
            return Ok(None);
        };

        let country_code = self.resolve_country_code(country)?;
        let asinfo = self.db.asinfo();

        if asinfo.is_empty() {
            if no_update {
                return Err(anyhow!(
                    "ASInfo data is empty but --country was requested. Re-run without --no-update or refresh ASInfo first."
                ));
            }
            self.db
                .refresh_asinfo()
                .map_err(|e| anyhow!("Failed to refresh ASInfo data for country filter: {}", e))?;
        } else if !no_update && asinfo.needs_refresh(self.config.asinfo_cache_ttl()) {
            self.db.refresh_asinfo().map_err(|e| {
                anyhow!(
                    "Failed to refresh stale ASInfo data for country filter: {}",
                    e
                )
            })?;
        }

        let mut asns = HashSet::new();
        let mut stmt = self
            .db
            .connection()
            .prepare("SELECT asn FROM asinfo_core WHERE UPPER(country) = UPPER(?1) ORDER BY asn")
            .map_err(|e| anyhow!("Failed to prepare ASInfo country lookup: {}", e))?;
        let rows = stmt
            .query_map([country_code.clone()], |row| row.get::<_, u32>(0))
            .map_err(|e| {
                anyhow!(
                    "Failed to query ASInfo by country '{}': {}",
                    country_code,
                    e
                )
            })?;

        for row in rows {
            asns.insert(row.map_err(|e| anyhow!("Failed to decode ASInfo country row: {}", e))?);
        }

        Ok(Some(asns))
    }

    fn resolve_country_code(&self, input: &str) -> Result<String> {
        let lens = CountryLens::new();
        let matches = lens.lookup(input);

        if matches.is_empty() {
            if input.len() == 2 {
                return Ok(input.to_uppercase());
            }
            return Err(anyhow!("Unknown country filter '{}'", input));
        }

        let exact_name_matches: Vec<_> = matches
            .iter()
            .filter(|entry| entry.name.eq_ignore_ascii_case(input))
            .collect();
        if exact_name_matches.len() == 1 {
            return Ok(exact_name_matches[0].code.clone());
        }

        let exact_code_matches: Vec<_> = matches
            .iter()
            .filter(|entry| entry.code.eq_ignore_ascii_case(input))
            .collect();
        if exact_code_matches.len() == 1 {
            return Ok(exact_code_matches[0].code.clone());
        }

        if matches.len() == 1 {
            return Ok(matches[0].code.clone());
        }

        Err(anyhow!(
            "Country filter '{}' is ambiguous; matches: {}",
            input,
            matches
                .iter()
                .map(|entry| format!("{} ({})", entry.name, entry.code))
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    fn parse_origin_filter(values: &[String]) -> Result<Option<OriginFilter>> {
        if values.is_empty() {
            return Ok(None);
        }

        let negated = values
            .first()
            .map(|value| value.starts_with('!'))
            .unwrap_or(false);
        let mut parsed = HashSet::new();

        for value in values {
            let asn = value
                .trim_start_matches('!')
                .parse::<u32>()
                .map_err(|e| anyhow!("Invalid origin ASN filter '{}': {}", value, e))?;
            parsed.insert(asn);
        }

        Ok(Some(OriginFilter {
            values: parsed,
            negated,
        }))
    }

    fn compile_as_path_regex(pattern: Option<&str>) -> Result<Option<Regex>> {
        pattern
            .map(|pattern| {
                Regex::new(pattern)
                    .map_err(|e| anyhow!("Invalid --as-path regex '{}': {}", pattern, e))
            })
            .transpose()
    }

    fn resolve_replay_groups(
        &self,
        args: &RibArgs,
        normalized_ts: &[i64],
    ) -> Result<Vec<RibReplayGroup>> {
        let first_ts = *normalized_ts
            .first()
            .ok_or_else(|| anyhow!("Missing earliest rib_ts after validation"))?;
        let last_ts = *normalized_ts
            .last()
            .ok_or_else(|| anyhow!("Missing latest rib_ts after validation"))?;

        // Use last_ts + 1 as ts_end because the broker's ts_end filter is exclusive.
        // Without this, a RIB dump that starts exactly at the target timestamp would
        // be excluded, forcing the code to use an earlier RIB and replay unnecessary
        // update files.
        let ribs = self
            .base_broker(args)
            .data_type("rib")
            .ts_start(Self::timestamp_to_broker_string(
                first_ts - Duration::hours(RIB_LOOKBACK_HOURS).num_seconds(),
            )?)
            .ts_end(Self::timestamp_to_broker_string(last_ts + 1)?)
            .query()
            .map_err(|e| anyhow!("Failed to query broker for candidate RIB files: {}", e))?;

        let mut ribs_by_collector: BTreeMap<String, Vec<BrokerItem>> = BTreeMap::new();
        for item in ribs {
            ribs_by_collector
                .entry(item.collector_id.clone())
                .or_default()
                .push(item);
        }

        let mut groups = Vec::new();
        for (collector, mut collector_ribs) in ribs_by_collector {
            collector_ribs.sort_by_key(|item| item.ts_start);

            let mut timestamps_by_rib: BTreeMap<String, (BrokerItem, Vec<i64>)> = BTreeMap::new();
            for rib_ts in normalized_ts {
                let selected_rib = collector_ribs
                    .iter()
                    .filter(|item| item.ts_start.and_utc().timestamp() <= *rib_ts)
                    .max_by_key(|item| item.ts_start);

                let Some(selected_rib) = selected_rib else {
                    return Err(anyhow!(
                        "No RIB file found at or before {} for collector {}",
                        Self::format_rib_ts_for_error(*rib_ts)?,
                        collector
                    ));
                };

                timestamps_by_rib
                    .entry(selected_rib.url.clone())
                    .and_modify(|(_, timestamps)| timestamps.push(*rib_ts))
                    .or_insert_with(|| (selected_rib.clone(), vec![*rib_ts]));
            }

            for (_, (rib_item, mut group_ts)) in timestamps_by_rib {
                group_ts.sort_unstable();
                let group_max_ts = *group_ts
                    .last()
                    .ok_or_else(|| anyhow!("Replay group was created without any rib_ts"))?;
                info!(
                    "Resolving updates for {}: {}s of updates",
                    collector,
                    group_max_ts - rib_item.ts_start.and_utc().timestamp()
                );
                let updates =
                    self.resolve_group_updates(args, &collector, &rib_item, group_max_ts)?;

                groups.push(RibReplayGroup {
                    collector: collector.clone(),
                    rib_item,
                    rib_ts: group_ts,
                    updates,
                });
            }
        }

        groups.sort_by(|a, b| {
            a.collector
                .cmp(&b.collector)
                .then(a.rib_item.ts_start.cmp(&b.rib_item.ts_start))
        });

        if groups.is_empty() {
            return Err(anyhow!(
                "No suitable RIB files were found for the requested timestamps and collector filters."
            ));
        }

        Ok(groups)
    }

    fn resolve_group_updates(
        &self,
        args: &RibArgs,
        collector: &str,
        rib_item: &BrokerItem,
        group_max_ts: i64,
    ) -> Result<Vec<BrokerItem>> {
        let rib_ts = rib_item.ts_start.and_utc().timestamp();

        let mut broker = self
            .base_broker(args)
            .collector_id(collector)
            .data_type("updates")
            .ts_start(Self::timestamp_to_broker_string(rib_ts)?)
            .ts_end(Self::timestamp_to_broker_string(group_max_ts)?);

        if let Some(project) = &args.filters.project {
            broker = broker.project(project);
        }

        let mut updates = broker.query().map_err(|e| {
            anyhow!(
                "Failed to query broker for updates for {}: {}",
                collector,
                e
            )
        })?;

        // Only keep update files that contain data up to and including the target timestamp.
        // An update file with ts_end <= group_max_ts has all elements with timestamp <= group_max_ts.
        updates.retain(|item| {
            let item_end = item.ts_end.and_utc().timestamp();
            item_end > rib_ts && item_end <= group_max_ts
        });
        updates.sort_by_key(|item| item.ts_start);
        Ok(updates)
    }

    fn build_full_feed_allowlists(&self, groups: &[RibReplayGroup]) -> Result<FullFeedAllowlists> {
        let mut allowlists = HashMap::new();

        for collector in groups
            .iter()
            .map(|group| group.collector.as_str())
            .collect::<BTreeSet<_>>()
        {
            let peers = BgpkitBroker::new()
                .collector_id(collector)
                .get_peers()
                .map_err(|e| {
                    anyhow!(
                        "Failed to fetch broker peer metadata for {}: {}",
                        collector,
                        e
                    )
                })?;

            let allowed = peers
                .into_iter()
                .filter(|peer| {
                    peer.num_v4_pfxs >= FULL_FEED_V4_THRESHOLD
                        || peer.num_v6_pfxs >= FULL_FEED_V6_THRESHOLD
                })
                .map(|peer| (peer.ip.to_string(), peer.asn))
                .collect::<HashSet<_>>();

            allowlists.insert(collector.to_string(), allowed);
        }

        Ok(allowlists)
    }

    #[allow(clippy::too_many_arguments)]
    fn load_base_rib(
        &self,
        state_store: &mut RibStateStore,
        collector: &str,
        rib_item: &BrokerItem,
        safe_filters: &ParseFilters,
        country_asns: Option<&HashSet<u32>>,
        origin_filter: Option<&OriginFilter>,
        as_path_regex: Option<&Regex>,
        full_feed_allowlist: Option<&HashSet<(String, u32)>>,
    ) -> Result<()> {
        let parser = safe_filters.to_parser(&rib_item.url).map_err(|e| {
            anyhow!(
                "Failed to build parser for base RIB {}: {}",
                rib_item.url,
                e
            )
        })?;

        let collector_arc = Arc::from(collector);
        let mut batch = Vec::new();
        let mut total = 0u64;
        for elem in parser {
            total += 1;
            info!("  parsed {} messages", total);
            if elem.elem_type != ElemType::ANNOUNCE {
                continue;
            }
            if self.announce_matches(
                collector,
                &elem,
                country_asns,
                origin_filter,
                as_path_regex,
                full_feed_allowlist,
            ) {
                batch.push(StoredRibEntry::from_elem(Arc::clone(&collector_arc), elem));
            }
        }

        state_store.upsert_entries(batch)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn replay_updates<F>(
        &self,
        state_store: &mut RibStateStore,
        group: &RibReplayGroup,
        args: &RibArgs,
        country_asns: Option<&HashSet<u32>>,
        origin_filter: Option<&OriginFilter>,
        as_path_regex: Option<&Regex>,
        full_feed_allowlist: Option<&HashSet<(String, u32)>>,
        snapshot_visitor: &mut F,
    ) -> Result<()>
    where
        F: FnMut(i64, &RibStateStore, &[StoredRibUpdate]) -> Result<()>,
    {
        let mut pending = HashMap::<RibRouteKey, DeltaOp>::new();
        let mut next_snapshot_index = 0usize;
        let collector_arc = Arc::from(group.collector.as_str());

        // Track filtered updates for the current snapshot interval
        // These are updates that matched filters and affected the RIB state
        let mut filtered_updates: Vec<StoredRibUpdate> = Vec::new();

        for (i, update) in group.updates.iter().enumerate() {
            info!(
                "  Replaying update file {}/{}: {}",
                i + 1,
                group.updates.len(),
                update.url
            );
            let safe_filters = self.safe_parse_filters(
                args,
                group.rib_item.ts_start.and_utc().timestamp(),
                *group
                    .rib_ts
                    .last()
                    .ok_or_else(|| anyhow!("Replay group missing max rib_ts"))?,
            );
            let parser = safe_filters.to_parser(&update.url).map_err(|e| {
                anyhow!(
                    "Failed to build parser for updates file {}: {}",
                    update.url,
                    e
                )
            })?;

            for elem in parser {
                while next_snapshot_index < group.rib_ts.len()
                    && elem.timestamp > group.rib_ts[next_snapshot_index] as f64
                {
                    self.flush_pending(state_store, &mut pending)?;
                    // For the first RIB (index 0), pass empty updates
                    // For subsequent RIBs, pass the collected filtered updates
                    snapshot_visitor(
                        group.rib_ts[next_snapshot_index],
                        state_store,
                        &filtered_updates,
                    )?;
                    // Clear updates after emitting snapshot (they belong to this snapshot)
                    filtered_updates.clear();
                    next_snapshot_index += 1;
                }

                // Apply update and track if it was filtered/matched
                let was_applied = self.apply_update_to_delta(
                    &mut pending,
                    state_store,
                    Arc::clone(&collector_arc),
                    &elem,
                    country_asns,
                    origin_filter,
                    as_path_regex,
                    full_feed_allowlist,
                )?;

                // If the update was applied (matched filters), track it for the updates table
                if was_applied {
                    let elem_type = elem.elem_type;
                    let update_record = StoredRibUpdate::from_elem(
                        group.rib_ts[next_snapshot_index.min(group.rib_ts.len() - 1)],
                        Arc::clone(&collector_arc),
                        elem,
                        elem_type,
                    );
                    filtered_updates.push(update_record);
                }
            }
        }

        while next_snapshot_index < group.rib_ts.len() {
            self.flush_pending(state_store, &mut pending)?;
            snapshot_visitor(
                group.rib_ts[next_snapshot_index],
                state_store,
                &filtered_updates,
            )?;
            filtered_updates.clear();
            next_snapshot_index += 1;
        }

        Ok(())
    }

    /// Apply an update to the pending delta and return whether it matched filters.
    ///
    /// Returns `true` if the update matched filters and was recorded in the delta,
    /// `false` if it was filtered out (doesn't mean it won't affect state - withdraws
    /// always check for existing routes).
    #[allow(clippy::too_many_arguments)]
    fn apply_update_to_delta(
        &self,
        pending: &mut HashMap<RibRouteKey, DeltaOp>,
        state_store: &RibStateStore,
        collector: Arc<str>,
        elem: &BgpElem,
        country_asns: Option<&HashSet<u32>>,
        origin_filter: Option<&OriginFilter>,
        as_path_regex: Option<&Regex>,
        full_feed_allowlist: Option<&HashSet<(String, u32)>>,
    ) -> Result<bool> {
        let route_key = RibRouteKey::from_elem(Arc::clone(&collector), elem);

        match elem.elem_type {
            ElemType::WITHDRAW => {
                if self.route_exists_in_state_or_delta(&route_key, state_store, pending)? {
                    pending.insert(route_key.clone(), DeltaOp::Delete(route_key));
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            ElemType::ANNOUNCE => {
                let matches = self.announce_matches(
                    &collector,
                    elem,
                    country_asns,
                    origin_filter,
                    as_path_regex,
                    full_feed_allowlist,
                );

                if matches {
                    pending.insert(
                        route_key,
                        DeltaOp::Upsert(StoredRibEntry::from_elem(collector, elem.clone())),
                    );
                    Ok(true)
                } else if self.route_exists_in_state_or_delta(&route_key, state_store, pending)? {
                    pending.insert(route_key.clone(), DeltaOp::Delete(route_key));
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    fn route_exists_in_state_or_delta(
        &self,
        route_key: &RibRouteKey,
        state_store: &RibStateStore,
        pending: &HashMap<RibRouteKey, DeltaOp>,
    ) -> Result<bool> {
        if let Some(delta) = pending.get(route_key) {
            return Ok(matches!(delta, DeltaOp::Upsert(_)));
        }
        state_store.route_exists(route_key)
    }

    fn flush_pending(
        &self,
        state_store: &mut RibStateStore,
        pending: &mut HashMap<RibRouteKey, DeltaOp>,
    ) -> Result<()> {
        if pending.is_empty() {
            return Ok(());
        }

        let mut upserts = Vec::new();
        let mut deletes = Vec::new();

        for delta in pending.values() {
            match delta {
                DeltaOp::Upsert(entry) => upserts.push(entry.clone()),
                DeltaOp::Delete(key) => deletes.push(key.clone()),
            }
        }

        if !upserts.is_empty() {
            state_store.upsert_entries(upserts)?;
        }
        if !deletes.is_empty() {
            state_store.delete_keys(deletes)?;
        }

        pending.clear();
        Ok(())
    }

    fn announce_matches(
        &self,
        collector: &str,
        elem: &BgpElem,
        country_asns: Option<&HashSet<u32>>,
        origin_filter: Option<&OriginFilter>,
        as_path_regex: Option<&Regex>,
        full_feed_allowlist: Option<&HashSet<(String, u32)>>,
    ) -> bool {
        if collector.is_empty() {
            return false;
        }

        if let Some(origin_filter) = origin_filter {
            let matches_origin = elem
                .origin_asns
                .as_ref()
                .map(|origins| {
                    origins
                        .iter()
                        .any(|asn| origin_filter.values.contains(&asn.to_u32()))
                })
                .unwrap_or(false);

            if origin_filter.negated {
                if matches_origin {
                    return false;
                }
            } else if !matches_origin {
                return false;
            }
        }

        if let Some(country_asns) = country_asns {
            let matches_country = elem
                .origin_asns
                .as_ref()
                .map(|origins| {
                    origins
                        .iter()
                        .any(|asn| country_asns.contains(&asn.to_u32()))
                })
                .unwrap_or(false);
            if !matches_country {
                return false;
            }
        }

        if let Some(as_path_regex) = as_path_regex {
            let as_path = elem
                .as_path
                .as_ref()
                .map(|path| path.to_string())
                .unwrap_or_default();
            if !as_path_regex.is_match(&as_path) {
                return false;
            }
        }

        if let Some(full_feed_allowlist) = full_feed_allowlist {
            let peer_key = (elem.peer_ip.to_string(), elem.peer_asn.to_u32());
            if !full_feed_allowlist.contains(&peer_key) {
                return false;
            }
        }

        true
    }

    /// Build filters for replaying BGP update files between two timestamps.
    ///
    /// Both `start_ts` and `end_ts` are applied so the parser only yields updates
    /// within the replay window.
    fn safe_parse_filters(&self, args: &RibArgs, start_ts: i64, end_ts: i64) -> ParseFilters {
        ParseFilters {
            prefix: args.filters.prefix.clone(),
            include_super: args.filters.include_super,
            include_sub: args.filters.include_sub,
            peer_asn: args.filters.peer_asn.clone(),
            start_ts: Some(start_ts.to_string()),
            end_ts: Some(end_ts.to_string()),
            ..Default::default()
        }
    }

    /// Build filters for loading a base RIB snapshot.
    ///
    /// No time filters are applied because RIB entries carry per-route timestamps
    /// (when the route was learned) that can be arbitrarily older than the RIB
    /// dump time.  Applying a `start_ts` filter at the dump time would incorrectly
    /// drop stable routes that were learned days or weeks earlier.
    fn safe_rib_filters(&self, args: &RibArgs) -> ParseFilters {
        ParseFilters {
            prefix: args.filters.prefix.clone(),
            include_super: args.filters.include_super,
            include_sub: args.filters.include_sub,
            peer_asn: args.filters.peer_asn.clone(),
            ..Default::default()
        }
    }

    fn base_broker(&self, args: &RibArgs) -> BgpkitBroker {
        let mut broker = BgpkitBroker::new().page_size(1000);
        if let Some(collector) = &args.filters.collector {
            broker = broker.collector_id(collector);
        }
        if let Some(project) = &args.filters.project {
            broker = broker.project(project);
        }
        broker
    }

    fn timestamp_to_broker_string(ts: i64) -> Result<String> {
        let timestamp = DateTime::from_timestamp(ts, 0)
            .ok_or_else(|| anyhow!("Invalid Unix timestamp {} for broker query", ts))?;
        Ok(timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string())
    }

    fn format_rib_ts_for_filename(rib_ts: i64) -> Result<String> {
        let timestamp = DateTime::from_timestamp(rib_ts, 0)
            .ok_or_else(|| anyhow!("Invalid Unix timestamp {} for file naming", rib_ts))?;
        Ok(timestamp.format("%Y%m%dT%H%M%SZ").to_string())
    }

    fn format_rib_ts_for_error(rib_ts: i64) -> Result<String> {
        let timestamp = DateTime::from_timestamp(rib_ts, 0)
            .ok_or_else(|| anyhow!("Invalid Unix timestamp {} for error reporting", rib_ts))?;
        Ok(timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string())
    }

    fn filter_slug(&self, filters: &RibFilters) -> Result<String> {
        let mut parts = Vec::new();

        if let Some(country) = &filters.country {
            parts.push(format!(
                "country-{}",
                Self::sanitize_slug_component(country)
            ));
        }
        if !filters.origin_asn.is_empty() {
            parts.push(format!(
                "origin-{}",
                Self::sanitize_list_component(&filters.origin_asn)
            ));
        }
        if !filters.peer_asn.is_empty() {
            parts.push(format!(
                "peer-{}",
                Self::sanitize_list_component(&filters.peer_asn)
            ));
        }
        if let Some(collector) = &filters.collector {
            let values = collector
                .split(',')
                .map(|value| value.trim().to_string())
                .collect::<Vec<_>>();
            parts.push(format!(
                "collector-{}",
                Self::sanitize_list_component(&values)
            ));
        }
        if let Some(project) = &filters.project {
            parts.push(format!(
                "project-{}",
                Self::sanitize_slug_component(project)
            ));
        }
        if !filters.prefix.is_empty() {
            parts.push(format!("prefix-{}", Self::hash8(&filters.prefix.join(","))));
        }
        if let Some(as_path) = &filters.as_path {
            parts.push(format!("aspath-{}", Self::hash8(as_path)));
        }
        if filters.full_feed_only {
            parts.push("fullfeed".to_string());
        }

        let slug = parts.join("-");
        if slug.len() <= 96 {
            return Ok(slug);
        }

        let truncated = slug
            .chars()
            .take(80)
            .collect::<String>()
            .trim_end_matches('-')
            .to_string();
        Ok(format!("{}-h{}", truncated, Self::hash8(&slug)))
    }

    fn sanitize_list_component(values: &[String]) -> String {
        let mut normalized = values
            .iter()
            .map(|value| Self::sanitize_slug_component(value))
            .collect::<Vec<_>>();
        normalized.sort();
        normalized.join("+")
    }

    fn sanitize_slug_component(input: &str) -> String {
        input
            .to_ascii_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
            .collect::<String>()
            .trim_matches('_')
            .to_string()
    }

    fn hash8(input: &str) -> String {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in input.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{:08x}", hash & 0xffff_ffff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_args() -> RibArgs {
        RibArgs {
            rib_ts: vec!["2025-09-01T12:00:00Z".to_string()],
            filters: RibFilters {
                ..Default::default()
            },
            sqlite_path: None,
        }
    }

    #[test]
    fn test_validate_multi_ts_stdout_error() {
        let mut args = base_args();
        args.rib_ts.push("2025-09-01T13:00:00Z".to_string());
        assert!(args.validate().is_err());
    }

    #[test]
    fn test_validate_multi_ts_file_output_ok() -> Result<()> {
        let mut args = base_args();
        args.rib_ts.push("2025-09-01T13:00:00Z".to_string());
        args.sqlite_path = Some(PathBuf::from("/tmp/monocle-rib.sqlite3"));
        let values = args.validate()?;
        assert_eq!(values.len(), 2);
        Ok(())
    }

    #[test]
    fn test_filter_slug_order() -> Result<()> {
        let mut args = base_args();
        args.filters.country = Some("IR".to_string());
        args.filters.origin_asn = vec!["15169".to_string(), "13335".to_string()];
        args.filters.peer_asn = vec!["2914".to_string()];
        args.filters.collector = Some("rrc00,route-views2".to_string());
        args.filters.project = Some("riperis".to_string());
        args.filters.prefix = vec!["1.1.1.0/24".to_string()];
        args.filters.as_path = Some("^15169 ".to_string());
        args.filters.full_feed_only = true;

        let db = MonocleDatabase::open_in_memory()?;
        let config = MonocleConfig::default();
        let lens = RibLens::new(&db, &config);
        let slug = lens.filter_slug(&args.filters)?;

        assert!(slug
            .starts_with("country-ir-origin-13335+15169-peer-2914-collector-route_views2+rrc00"));
        assert!(slug.contains("-h"));
        Ok(())
    }

    #[test]
    fn test_hash8_is_stable() {
        assert_eq!(RibLens::hash8("a"), RibLens::hash8("a"));
    }

    #[test]
    fn test_file_name_prefix_includes_filters() -> Result<()> {
        let mut args = base_args();
        args.filters.country = Some("US".to_string());
        args.filters.origin_asn = vec!["13335".to_string()];
        args.filters.full_feed_only = true;

        let db = MonocleDatabase::open_in_memory()?;
        let config = MonocleConfig::default();
        let lens = RibLens::new(&db, &config);
        let file_name = format!(
            "{}.sqlite3",
            lens.file_name_prefix(&args, &[1_756_728_000])?
        );

        assert_eq!(
            file_name,
            "monocle-rib-20250901T120000Z-country-us-origin-13335-fullfeed.sqlite3"
        );
        Ok(())
    }
}
