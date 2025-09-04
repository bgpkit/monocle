use anyhow::anyhow;
use chrono::{DateTime, TimeZone, Utc};
use chrono_humanize::HumanTime;
use tabled::settings::Style;
use tabled::{Table, Tabled};

#[derive(Tabled)]
struct BgpTime {
    unix: i64,
    rfc3339: String,
    human: String,
}

pub fn string_to_time(time_string: &str) -> anyhow::Result<DateTime<Utc>> {
    let ts = match dateparser::parse_with(
        time_string,
        &Utc,
        chrono::NaiveTime::from_hms_opt(0, 0, 0).ok_or_else(|| anyhow!("Failed to create time"))?,
    ) {
        Ok(ts) => ts,
        Err(_) => {
            return Err(anyhow!(
                "Input time must be either Unix timestamp or time string compliant with RFC3339"
            ))
        }
    };

    Ok(ts)
}

pub fn parse_time_string_to_rfc3339(time_vec: &[String]) -> anyhow::Result<String> {
    let mut time_strings = vec![];
    if time_vec.is_empty() {
        time_strings.push(Utc::now().to_rfc3339())
    } else {
        for ts in time_vec {
            match string_to_time(ts) {
                Ok(ts) => time_strings.push(ts.to_rfc3339()),
                Err(_) => return Err(anyhow!("unable to parse timestring: {}", ts)),
            }
        }
    }

    Ok(time_strings.join("\n"))
}

pub fn time_to_table(time_vec: &[String]) -> anyhow::Result<String> {
    let now_ts = Utc::now().timestamp();
    let ts_vec = match time_vec.is_empty() {
        true => vec![now_ts],
        false => time_vec
            .iter()
            .map(|ts| string_to_time(ts.as_str()).map(|dt| dt.timestamp()))
            .collect::<anyhow::Result<Vec<_>>>()?,
    };

    let bgptime_vec = ts_vec
        .into_iter()
        .map(|ts| {
            let ht = HumanTime::from(chrono::Local::now() - chrono::Duration::seconds(now_ts - ts));
            let human = ht.to_string();
            let rfc3339 = Utc
                .from_utc_datetime(
                    &DateTime::from_timestamp(ts, 0)
                        .unwrap_or_default()
                        .naive_utc(),
                )
                .to_rfc3339();
            BgpTime {
                unix: ts,
                rfc3339,
                human,
            }
        })
        .collect::<Vec<BgpTime>>();

    Ok(Table::new(bgptime_vec).with(Style::rounded()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_time() {
        use chrono::TimeZone;

        // Test with a valid Unix timestamp
        let unix_ts = "1697043600"; // Example timestamp
        let result = string_to_time(unix_ts);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Utc.timestamp_opt(1697043600, 0).unwrap());

        // Test with a valid RFC3339 string
        let rfc3339_str = "2023-10-11T00:00:00Z";
        let result = string_to_time(rfc3339_str);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Utc.timestamp_opt(1696982400, 0).unwrap());

        // Test with an incorrect date string
        let invalid_date = "not-a-date";
        let result = string_to_time(invalid_date);
        assert!(result.is_err());

        // Test with an empty string
        let empty_string = "";
        let result = string_to_time(empty_string);
        assert!(result.is_err());

        // Test with incomplete RFC3339 string
        let incomplete_rfc3339 = "2023-10-11T";
        let result = string_to_time(incomplete_rfc3339);
        assert!(result.is_err());

        // Test with a human-readable date string allowed by `dateparser`
        let human_readable = "October 11, 2023";
        let result = string_to_time(human_readable);
        assert!(result.is_ok());
        let expected_time = Utc.with_ymd_and_hms(2023, 10, 11, 0, 0, 0).unwrap();
        assert_eq!(result.unwrap(), expected_time);
    }
}
