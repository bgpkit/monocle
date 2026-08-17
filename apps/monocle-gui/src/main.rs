use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_channel::{Receiver, Sender};
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    button::{Button, ButtonVariants as _},
    input::{Input, InputState},
    progress::Progress,
    table::{Column, DataTable, TableDelegate, TableState},
    ActiveTheme as _, Disableable as _, Root, Sizable as _, StyledExt as _,
};
use monocle::lens::parse::ParseFilters;
use monocle::lens::search::{
    SearchControl, SearchDumpType, SearchElementBatch, SearchExecutionOptions, SearchFilters,
    SearchLens, SearchOutcome, SearchProgress, SearchSink,
};

const DEFAULT_MAX_RESULTS: u64 = 500;

#[derive(Clone)]
struct SearchRow {
    elem_type: SharedString,
    timestamp: SharedString,
    collector: SharedString,
    peer_asn: SharedString,
    peer_ip: SharedString,
    prefix: SharedString,
    as_path: SharedString,
    origin_asns: SharedString,
    next_hop: SharedString,
}

impl SearchRow {
    fn from_batch(batch: SearchElementBatch) -> Vec<Self> {
        let collector: SharedString = batch.collector.into();
        batch
            .elements
            .into_iter()
            .map(|elem| {
                let elem_type = format!("{:?}", elem.elem_type);
                let origin_asns = elem
                    .origin_asns
                    .as_ref()
                    .map(|values| {
                        values
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();

                Self {
                    elem_type: elem_type.into(),
                    timestamp: format!("{:.3}", elem.timestamp).into(),
                    collector: collector.clone(),
                    peer_asn: elem.peer_asn.to_string().into(),
                    peer_ip: elem.peer_ip.to_string().into(),
                    prefix: elem.prefix.to_string().into(),
                    as_path: elem
                        .as_path
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default()
                        .into(),
                    origin_asns: origin_asns.into(),
                    next_hop: elem
                        .next_hop
                        .map(|value| value.to_string())
                        .unwrap_or_default()
                        .into(),
                }
            })
            .collect()
    }
}

struct ResultsTable {
    columns: Vec<Column>,
    rows: Vec<SearchRow>,
}

impl ResultsTable {
    fn new() -> Self {
        Self {
            columns: vec![
                Column::new("type", "Type").width(px(78.)),
                Column::new("timestamp", "Timestamp").width(px(150.)),
                Column::new("collector", "Collector").width(px(130.)),
                Column::new("peer-asn", "Peer ASN").width(px(110.)),
                Column::new("peer-ip", "Peer IP").width(px(150.)),
                Column::new("prefix", "Prefix").width(px(160.)),
                Column::new("origin", "Origin ASN").width(px(130.)),
                Column::new("next-hop", "Next hop").width(px(150.)),
                Column::new("as-path", "AS path").width(px(420.)),
            ],
            rows: Vec::new(),
        }
    }
}

impl TableDelegate for ResultsTable {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        self.columns[col_ix].clone()
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let row = &self.rows[row_ix];
        match col_ix {
            0 => row.elem_type.clone(),
            1 => row.timestamp.clone(),
            2 => row.collector.clone(),
            3 => row.peer_asn.clone(),
            4 => row.peer_ip.clone(),
            5 => row.prefix.clone(),
            6 => row.origin_asns.clone(),
            7 => row.next_hop.clone(),
            8 => row.as_path.clone(),
            _ => SharedString::default(),
        }
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let row = &self.rows[row_ix];
        match col_ix {
            0 => row.elem_type.to_string(),
            1 => row.timestamp.to_string(),
            2 => row.collector.to_string(),
            3 => row.peer_asn.to_string(),
            4 => row.peer_ip.to_string(),
            5 => row.prefix.to_string(),
            6 => row.origin_asns.to_string(),
            7 => row.next_hop.to_string(),
            8 => row.as_path.to_string(),
            _ => String::new(),
        }
    }
}

enum SearchEvent {
    Progress(SearchProgress),
    Rows(Vec<SearchRow>),
    Finished(Result<SearchOutcome, String>),
}

struct GuiSearchSink {
    sender: Sender<SearchEvent>,
}

impl SearchSink for GuiSearchSink {
    fn on_progress(&self, progress: SearchProgress) {
        let _ = self.sender.send_blocking(SearchEvent::Progress(progress));
    }

    fn on_elements(&self, batch: SearchElementBatch) -> SearchControl {
        if self
            .sender
            .send_blocking(SearchEvent::Rows(SearchRow::from_batch(batch)))
            .is_ok()
        {
            SearchControl::Continue
        } else {
            SearchControl::Stop
        }
    }
}

struct SearchApp {
    start_input: Entity<InputState>,
    end_input: Entity<InputState>,
    collector_input: Entity<InputState>,
    prefix_input: Entity<InputState>,
    origin_input: Entity<InputState>,
    max_results_input: Entity<InputState>,
    dump_type: SearchDumpType,
    table: Entity<TableState<ResultsTable>>,
    running: bool,
    progress: f32,
    status: SharedString,
    result_count: usize,
    file_count: usize,
    cancel_flag: Option<Arc<AtomicBool>>,
    search_task: Option<Task<()>>,
}

impl SearchApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let start = now.saturating_sub(2 * 60 * 60);
        let end = start + 15 * 60;

        let start_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Start time")
                .default_value(start.to_string())
        });
        let end_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("End time")
                .default_value(end.to_string())
        });
        let collector_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("rrc00 (optional)"));
        let prefix_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("1.1.1.0/24, 8.8.8.0/24"));
        let origin_input = cx.new(|cx| InputState::new(window, cx).placeholder("13335, 15169"));
        let max_results_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Result limit")
                .default_value(DEFAULT_MAX_RESULTS.to_string())
        });
        let table = cx.new(|cx| {
            TableState::new(ResultsTable::new(), window, cx)
                .sortable(false)
                .col_movable(false)
                .row_selectable(true)
        });

        Self {
            start_input,
            end_input,
            collector_input,
            prefix_input,
            origin_input,
            max_results_input,
            dump_type: SearchDumpType::Updates,
            table,
            running: false,
            progress: 0.,
            status: "Ready to search public MRT archives".into(),
            result_count: 0,
            file_count: 0,
            cancel_flag: None,
            search_task: None,
        }
    }

    fn input_value(input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_string()
    }

    fn csv_values(value: String) -> Vec<String> {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn build_query(&self, cx: &App) -> Result<(SearchFilters, u64), String> {
        let start = Self::input_value(&self.start_input, cx);
        let end = Self::input_value(&self.end_input, cx);
        let collector = Self::input_value(&self.collector_input, cx);
        let max_results = Self::input_value(&self.max_results_input, cx)
            .parse::<u64>()
            .map_err(|_| "Result limit must be a positive integer".to_string())?;
        if max_results == 0 {
            return Err("Result limit must be greater than zero".to_string());
        }

        let filters = SearchFilters {
            parse_filters: ParseFilters {
                start_ts: Some(start),
                end_ts: Some(end),
                prefix: Self::csv_values(Self::input_value(&self.prefix_input, cx)),
                origin_asn: Self::csv_values(Self::input_value(&self.origin_input, cx)),
                ..Default::default()
            },
            collector: (!collector.is_empty()).then_some(collector),
            dump_type: self.dump_type.clone(),
            ..Default::default()
        };
        filters.validate().map_err(|error| error.to_string())?;
        Ok((filters, max_results))
    }

    fn start_search(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }

        let (filters, max_results) = match self.build_query(cx) {
            Ok(query) => query,
            Err(error) => {
                self.status = error.into();
                cx.notify();
                return;
            }
        };

        self.table.update(cx, |table, cx| {
            table.delegate_mut().rows.clear();
            table.refresh(cx);
        });
        self.running = true;
        self.progress = 0.;
        self.result_count = 0;
        self.file_count = 0;
        self.status = "Querying BGPKIT Broker…".into();

        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.cancel_flag = Some(cancel_flag.clone());
        let (sender, receiver) = async_channel::bounded(32);
        let sink = Arc::new(GuiSearchSink {
            sender: sender.clone(),
        });

        std::thread::spawn(move || {
            let options = SearchExecutionOptions {
                max_results: Some(max_results),
                cancel_flag: Some(cancel_flag),
                batch_size: 128,
                ..Default::default()
            };
            let result = SearchLens::new()
                .search_with_options(&filters, options, sink)
                .map_err(|error| error.to_string());
            let _ = sender.send_blocking(SearchEvent::Finished(result));
        });

        self.search_task = Some(cx.spawn(async move |this, cx| {
            Self::consume_events(this, receiver, cx).await;
        }));
        cx.notify();
    }

    async fn consume_events(
        this: WeakEntity<Self>,
        receiver: Receiver<SearchEvent>,
        cx: &mut AsyncApp,
    ) {
        while let Ok(event) = receiver.recv().await {
            let terminal = matches!(event, SearchEvent::Finished(_));
            if this
                .update(cx, |this, cx| this.handle_event(event, cx))
                .is_err()
            {
                break;
            }
            if terminal {
                break;
            }
        }
    }

    fn handle_event(&mut self, event: SearchEvent, cx: &mut Context<Self>) {
        match event {
            SearchEvent::Rows(rows) => {
                self.result_count += rows.len();
                self.table.update(cx, |table, cx| {
                    table.delegate_mut().rows.extend(rows);
                    table.refresh(cx);
                });
            }
            SearchEvent::Progress(progress) => match progress {
                SearchProgress::QueryingBroker => {
                    self.status = "Querying BGPKIT Broker…".into();
                }
                SearchProgress::FilesFound { count } => {
                    self.file_count = count;
                    self.status = format!("Searching {count} MRT files").into();
                }
                SearchProgress::FileStarted {
                    file_index,
                    total_files,
                    collector,
                    ..
                } => {
                    self.status = format!(
                        "Processing file {} of {} from {}",
                        file_index + 1,
                        total_files,
                        collector
                    )
                    .into();
                }
                SearchProgress::ProgressUpdate {
                    percent_complete,
                    total_messages,
                    ..
                } => {
                    self.progress = percent_complete as f32;
                    self.status =
                        format!("{percent_complete:.0}% complete · {total_messages} matches")
                            .into();
                }
                SearchProgress::Completed { .. } | SearchProgress::FileCompleted { .. } => {}
            },
            SearchEvent::Finished(result) => {
                self.running = false;
                self.cancel_flag = None;
                self.progress = 100.;
                self.status = match result {
                    Ok(outcome) => format!(
                        "{:?} · {} results · {}/{} files · {:.1}s",
                        outcome.exit_reason,
                        outcome.summary.total_messages,
                        outcome.summary.successful_files,
                        outcome.summary.total_files,
                        outcome.summary.duration_secs
                    )
                    .into(),
                    Err(error) => format!("Search failed: {error}").into(),
                };
            }
        }
        cx.notify();
    }

    fn cancel_search(&mut self, cx: &mut Context<Self>) {
        if let Some(flag) = &self.cancel_flag {
            flag.store(true, Ordering::Relaxed);
            self.status = "Cancelling after the active parser batch…".into();
            cx.notify();
        }
    }

    fn set_dump_type(&mut self, dump_type: SearchDumpType, cx: &mut Context<Self>) {
        if !self.running {
            self.dump_type = dump_type;
            cx.notify();
        }
    }

    fn field(&self, label: &'static str, input: &Entity<InputState>, cx: &App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(Input::new(input).disabled(self.running))
    }
}

impl Render for SearchApp {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updates_selected = self.dump_type == SearchDumpType::Updates;
        let rib_selected = self.dump_type == SearchDumpType::Rib;
        let all_selected = self.dump_type == SearchDumpType::RibUpdates;

        let filters = div()
            .w(px(330.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .gap_4()
            .p_5()
            .border_r_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_xl().font_semibold().child("Monocle Search"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Explore public BGP MRT archives"),
                    ),
            )
            .child(self.field("START TIME", &self.start_input, cx))
            .child(self.field("END TIME", &self.end_input, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_medium()
                            .text_color(cx.theme().muted_foreground)
                            .child("DUMP TYPE"),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("updates")
                                    .label("Updates")
                                    .small()
                                    .disabled(self.running)
                                    .when(updates_selected, |button| button.primary())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_dump_type(SearchDumpType::Updates, cx)
                                    })),
                            )
                            .child(
                                Button::new("rib")
                                    .label("RIB")
                                    .small()
                                    .disabled(self.running)
                                    .when(rib_selected, |button| button.primary())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_dump_type(SearchDumpType::Rib, cx)
                                    })),
                            )
                            .child(
                                Button::new("all")
                                    .label("Both")
                                    .small()
                                    .disabled(self.running)
                                    .when(all_selected, |button| button.primary())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_dump_type(SearchDumpType::RibUpdates, cx)
                                    })),
                            ),
                    ),
            )
            .child(self.field("COLLECTOR", &self.collector_input, cx))
            .child(self.field("PREFIXES", &self.prefix_input, cx))
            .child(self.field("ORIGIN ASNS", &self.origin_input, cx))
            .child(self.field("RESULT LIMIT", &self.max_results_input, cx))
            .child(div().flex_1())
            .child(
                Button::new("search")
                    .primary()
                    .w_full()
                    .label(if self.running {
                        "Searching…"
                    } else {
                        "Search"
                    })
                    .disabled(self.running)
                    .on_click(cx.listener(|this, _, _, cx| this.start_search(cx))),
            )
            .when(self.running, |panel| {
                panel.child(
                    Button::new("cancel")
                        .outline()
                        .w_full()
                        .label("Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_search(cx))),
                )
            });

        let content = div()
            .flex_1()
            .h_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_5()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(self.status.clone())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} visible rows · {} matching files",
                                        self.result_count, self.file_count
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .w(px(220.))
                            .child(Progress::new("search-progress").value(self.progress)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).stripe(true).bordered(false)),
            );

        div()
            .size_full()
            .flex()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(filters)
            .child(content)
    }
}

fn main() {
    gpui_platform::application().run(|cx| {
        gpui_component::init(cx);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(780.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            let result = cx.open_window(options, |window, cx| {
                window.set_window_title("Monocle Search");
                let view = cx.new(|cx| SearchApp::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            });
            if let Err(error) = result {
                eprintln!("failed to open Monocle window: {error}");
            }
        })
        .detach();
    });
}
