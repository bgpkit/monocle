use crate::filters::parse::ParseFilters;
use crate::filters::MrtParserFilters;
use anyhow::Result;
use bgpkit_broker::BrokerItem;
use bgpkit_parser::BgpkitParser;
use clap::{Args, ValueEnum};
use serde::Serialize;
use std::io::Read;

#[derive(Args, Debug, Clone)]
pub struct SearchFilters {
    #[clap(flatten)]
    pub parse_filters: ParseFilters,

    /// Filter by collector, e.g., rrc00 or route-views2
    #[clap(short = 'c', long)]
    pub collector: Option<String>,

    /// Filter by route collection project, i.e., riperis or routeviews
    #[clap(short = 'P', long)]
    pub project: Option<String>,

    /// Specify data dump type to search (updates or RIB dump)
    #[clap(short = 'D', long, default_value_t, value_enum)]
    pub dump_type: DumpType,
}

#[derive(ValueEnum, Clone, Debug, Default, Serialize)]
pub enum DumpType {
    /// BGP updates only
    #[default]
    Updates,
    /// BGP RIB dump only
    Rib,
    /// BGP RIB dump and BGP updates
    RibUpdates,
}

impl SearchFilters {
    pub fn to_broker_items(&self) -> Result<Vec<BrokerItem>> {
        // it's fine to unwrap as the filters.validate() function has already checked for issues
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
            DumpType::Updates => {
                broker = broker.data_type("updates");
            }
            DumpType::Rib => {
                broker = broker.data_type("rib");
            }
            DumpType::RibUpdates => {
                // do nothing here -> getting all RIB and updates
            }
        }

        broker
            .query()
            .map_err(|_| anyhow::anyhow!("broker query error: please check filters are valid"))
    }
}

impl MrtParserFilters for SearchFilters {
    fn validate(&self) -> Result<()> {
        let _ = self.parse_filters.parse_start_end_strings()?;
        Ok(())
    }

    fn to_parser(&self, file_path: &str) -> Result<BgpkitParser<Box<dyn Read + Send>>> {
        self.parse_filters.to_parser(file_path)
    }
}
