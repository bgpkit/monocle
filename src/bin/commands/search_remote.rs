//! Remote search client — consumes SSE from a Monocle HTTP service and
//! formats results using the same output formatters as local search.
//!
//! Used when `--remote-url` is passed to `monocle search`. The client POSTs
//! the search filters as JSON and reads the `text/event-stream` response,
//! parsing SSE frames to extract `elements` events containing `BgpElem` values.

use std::collections::HashMap;

use bgpkit_parser::BgpElem;
use futures::StreamExt;
use monocle::utils::{OutputFormat, TimestampFormat};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncBufReadExt;

use super::elem_format::{format_elem, format_elems_table};

/// Wire DTO matching the server's `SearchStreamRequest`.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteSearchRequest {
    pub filters: RemoteSearchFilters,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u64>,
}

/// Wire filters — field names match the server's `SearchStreamFilters`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RemoteSearchFilters {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prefix: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub include_super: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub include_sub: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub origin_asn: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub peer_asn: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub peer_ip: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub communities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elem_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_path: Option<String>,
    pub start_ts: String,
    pub end_ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dump_type: Option<String>,
}

/// SSE event data for `elements` events.
#[derive(Debug, Clone, Deserialize)]
pub struct ElementsBatch {
    #[allow(dead_code)]
    pub total_so_far: u64,
    pub collector: Option<String>,
    pub elements: Vec<BgpElem>,
}

/// Progress event (loosely typed — we just print it).
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum ProgressData {
    Simple(String),
    Fields(HashMap<String, serde_json::Value>),
}

/// Summary for `completed` events.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchSummary {
    pub total_files: usize,
    pub successful_files: usize,
    pub failed_files: usize,
    pub total_messages: u64,
    pub duration_secs: f64,
}

/// Run a remote search: POST to the server, consume SSE, format elements.
pub async fn run_remote_search(
    url: &str,
    auth_token: Option<&str>,
    request: RemoteSearchRequest,
    output_format: OutputFormat,
    fields: &[&str],
    time_format: TimestampFormat,
) -> anyhow::Result<()> {
    let mut client_builder = reqwest::Client::builder();
    let _ = &mut client_builder; // suppress unused mut warning
    let client = client_builder.build()?;

    let mut req = client
        .post(url)
        .header("Accept", "text/event-stream")
        .header("Content-Type", "application/json")
        .json(&request);

    if let Some(token) = auth_token {
        req = req.header("Authorization", format!("Bearer {}", token));
    }

    let response = req.send().await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Remote search failed (HTTP {}): {}", status, body);
    }

    // Read the response body as a stream and parse SSE frames
    let byte_stream = response.bytes_stream();
    let mut reader = tokio_util::io::StreamReader::new(
        byte_stream.map(|chunk| chunk.map_err(std::io::Error::other)),
    );
    let mut lines = tokio::io::BufReader::new(&mut reader).lines();

    let mut buffered_elems: Vec<(BgpElem, Option<String>)> = Vec::new();
    let is_table = output_format == OutputFormat::Table;

    // For non-table formats that need a header (markdown)
    if output_format == OutputFormat::Markdown {
        println!("| {} |", fields.join(" | "));
        println!(
            "|{}|",
            fields.iter().map(|_| "---").collect::<Vec<_>>().join("|")
        );
    }

    while let Ok(Some(line)) = lines.next_line().await {
        if line.is_empty() {
            continue;
        }

        // Parse SSE: "event: <type>" and "data: <json>"
        if let Some(event_type) = line.strip_prefix("event: ") {
            // Read the data line that follows
            let data_line = match lines.next_line().await {
                Ok(Some(dl)) if dl.starts_with("data: ") => {
                    dl.strip_prefix("data: ").unwrap_or("").to_string()
                }
                _ => continue,
            };

            match event_type {
                "started" => {
                    // Could print start info; skip for now
                }
                "progress" => {
                    if let Ok(data) = serde_json::from_str::<ProgressData>(&data_line) {
                        eprintln!(
                            "[progress] {}",
                            serde_json::to_string(&data).unwrap_or_default()
                        );
                    }
                }
                "elements" => {
                    if let Ok(batch) = serde_json::from_str::<ElementsBatch>(&data_line) {
                        for elem in &batch.elements {
                            if is_table {
                                buffered_elems.push((elem.clone(), batch.collector.clone()));
                            } else if let Some(output_str) = format_elem(
                                elem,
                                output_format,
                                fields,
                                batch.collector.as_deref(),
                                time_format,
                            ) {
                                println!("{}", output_str);
                            }
                        }
                    }
                }
                "completed" => {
                    if let Ok(summary) = serde_json::from_str::<SearchSummary>(&data_line) {
                        eprintln!(
                            "[completed] {} files ({} ok, {} failed), {} messages in {:.2}s",
                            summary.total_files,
                            summary.successful_files,
                            summary.failed_files,
                            summary.total_messages,
                            summary.duration_secs
                        );
                    }
                    break;
                }
                "cancelled" => {
                    eprintln!("[cancelled]");
                    break;
                }
                "error" => {
                    eprintln!("[error] {}", data_line);
                    break;
                }
                _ => {}
            }
        }
    }

    // Flush table output
    if is_table && !buffered_elems.is_empty() {
        let table = format_elems_table(&buffered_elems, fields, time_format);
        println!("{}", table);
    }

    Ok(())
}
