//! Cloudflare Radar data access.
//!
//! Note: using this module requires setting CF_API_TOKEN in environment variables

use radar_rs::{PrefixOriginsResult, RadarClient, RadarError, RoutingStatsResult};

pub struct CfRadar {
    client: RadarClient,
}

impl CfRadar {
    pub fn new() -> Result<Self, RadarError> {
        Ok(Self {
            client: RadarClient::new()?,
        })
    }

    pub fn get_bgp_routing_stats(
        &self,
        asn: Option<u32>,
        country_code: Option<String>,
    ) -> Result<RoutingStatsResult, RadarError> {
        self.client.get_bgp_routing_stats(asn, country_code)
    }

    pub fn get_prefix_origins(
        &self,
        origin: Option<u32>,
        prefix: Option<String>,
        rpki_status: Option<String>,
    ) -> Result<PrefixOriginsResult, RadarError> {
        self.client
            .get_bgp_prefix_origins(origin, prefix, rpki_status)
    }
}
