pub use parse::ParseFilters;
pub use search::SearchFilters;

use bgpkit_parser::BgpkitParser;
use clap::ValueEnum;
use serde::Serialize;
use std::fmt::Display;
use std::io::Read;

mod parse;
mod search;

#[derive(ValueEnum, Clone, Debug, Serialize)]
pub enum ElemTypeEnum {
    /// BGP announcement
    A,
    /// BGP withdrawal
    W,
}

impl Display for ElemTypeEnum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ElemTypeEnum::A => "announcement",
            ElemTypeEnum::W => "withdrawal",
        })
    }
}

pub trait MrtParserFilters {
    fn validate(&self) -> anyhow::Result<()>;
    fn to_parser(&self, path: &str) -> anyhow::Result<BgpkitParser<Box<dyn Read + Send>>>;
}
