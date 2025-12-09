use clap::Args;
use monocle::{As2org, MonocleConfig, SearchResult, SearchResultConcise, SearchType};
use tabled::settings::Style;
use tabled::Table;

/// Arguments for the Whois command
#[derive(Args)]
pub struct WhoisArgs {
    /// Search query, an ASN (e.g. "400644") or a name (e.g. "bgpkit")
    pub query: Vec<String>,

    /// Search AS and Org name only
    #[clap(short, long)]
    pub name_only: bool,

    /// Search by ASN only
    #[clap(short, long)]
    pub asn_only: bool,

    /// Search by country only
    #[clap(short = 'C', long)]
    pub country_only: bool,

    /// Refresh the local as2org database
    #[clap(short, long)]
    pub update: bool,

    /// Output to pretty table, default markdown table
    #[clap(short, long)]
    pub pretty: bool,

    /// Display a full table (with ord_id, org_size)
    #[clap(short = 'F', long)]
    pub full_table: bool,

    /// Export to pipe-separated values
    #[clap(short = 'P', long)]
    pub psv: bool,

    /// Show full country names instead of 2-letter code
    #[clap(short, long)]
    pub full_country: bool,
}

pub fn run(config: &MonocleConfig, args: WhoisArgs) {
    let WhoisArgs {
        query,
        name_only,
        asn_only,
        country_only,
        update,
        pretty,
        full_table,
        full_country,
        psv,
    } = args;

    let data_dir = config.data_dir.as_str();
    let as2org = match As2org::new(&Some(format!("{data_dir}/monocle-data.sqlite3"))) {
        Ok(as2org) => as2org,
        Err(e) => {
            eprintln!("Failed to create AS2org database: {}", e);
            std::process::exit(1);
        }
    };

    if update {
        // if the update flag is set, clear existing as2org data and re-download later
        if let Err(e) = as2org.clear_db() {
            eprintln!("Failed to clear database: {}", e);
            std::process::exit(1);
        }
    }

    if as2org.is_db_empty() {
        println!("bootstrapping as2org data now... (it will take about one minute)");
        if let Err(e) = as2org.parse_insert_as2org(None) {
            eprintln!("Failed to bootstrap AS2org data: {}", e);
            std::process::exit(1);
        }
        println!("bootstrapping as2org data finished");
    }

    let mut search_type: SearchType = match (name_only, asn_only) {
        (true, false) => SearchType::NameOnly,
        (false, true) => SearchType::AsnOnly,
        (false, false) => SearchType::Guess,
        (true, true) => {
            eprintln!("ERROR: name-only and asn-only cannot be both true");
            return;
        }
    };

    if country_only {
        search_type = SearchType::CountryOnly;
    }

    let mut res = query
        .into_iter()
        .flat_map(
            |q| match as2org.search(q.as_str(), &search_type, full_country) {
                Ok(results) => results,
                Err(e) => {
                    eprintln!("Search error for '{}': {}", q, e);
                    Vec::new()
                }
            },
        )
        .collect::<Vec<SearchResult>>();

    // order search results by AS number
    res.sort_by_key(|v| v.asn);

    match full_table {
        false => {
            let res_concise = res.into_iter().map(|x: SearchResult| SearchResultConcise {
                asn: x.asn,
                as_name: x.as_name,
                org_name: x.org_name,
                org_country: x.org_country,
            });
            if psv {
                println!("asn|asn_name|org_name|org_country");
                for res in res_concise {
                    println!(
                        "{}|{}|{}|{}",
                        res.asn, res.as_name, res.org_name, res.org_country
                    );
                }
                return;
            }

            match pretty {
                true => {
                    println!("{}", Table::new(res_concise).with(Style::rounded()));
                }
                false => {
                    println!("{}", Table::new(res_concise).with(Style::markdown()));
                }
            };
        }
        true => {
            if psv {
                println!("asn|asn_name|org_name|org_id|org_country|org_size");
                for entry in res {
                    println!(
                        "{}|{}|{}|{}|{}|{}",
                        entry.asn,
                        entry.as_name,
                        entry.org_name,
                        entry.org_id,
                        entry.org_country,
                        entry.org_size
                    );
                }
                return;
            }
            match pretty {
                true => {
                    println!("{}", Table::new(res).with(Style::rounded()));
                }
                false => {
                    println!("{}", Table::new(res).with(Style::markdown()));
                }
            };
        }
    }
}
