//! RTR (RPKI-to-Router) client implementation.
//!
//! This module provides a client for fetching ROA data from RTR servers
//! using the RPKI-to-Router Protocol (RFC 8210, version 1).
//!
//! Note: RTR v1 only supports ROAs and Router Keys. ASPA support requires
//! RTR v2 (draft-ietf-sidrops-8210bis), which is not yet standardized.
//!
//! # Example
//!
//! ```rust,ignore
//! use monocle::lens::rpki::rtr::RtrClient;
//! use std::time::Duration;
//!
//! let client = RtrClient::new(
//!     "rtr.rpki.cloudflare.com".to_string(),
//!     8282,
//!     Duration::from_secs(60),
//! );
//!
//! let roas = client.fetch_roas()?;
//! println!("Fetched {} ROAs", roas.len());
//! ```

use anyhow::{anyhow, Result};
use bgpkit_parser::models::rpki::rtr::*;
use bgpkit_parser::parser::rpki::rtr::{read_rtr_pdu, RtrEncode};
use std::io::Write;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;
use tracing::{info, warn};

use crate::database::RpkiRoaRecord;

/// RTR client for fetching ROA data from an RTR server.
///
/// This client implements the RPKI-to-Router Protocol (RFC 8210) to fetch
/// Validated ROA Payloads (VRPs) from an RTR cache server.
pub struct RtrClient {
    host: String,
    port: u16,
    timeout: Duration,
}

impl RtrClient {
    /// Create a new RTR client.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address of the RTR server
    /// * `port` - The port number (typically 8282 for RTR)
    /// * `timeout` - Connection and read/write timeout
    pub fn new(host: String, port: u16, timeout: Duration) -> Self {
        Self {
            host,
            port,
            timeout,
        }
    }

    /// Fetch all ROAs from the RTR server.
    ///
    /// Connects to the server, sends a Reset Query to request the full
    /// database, and collects all IPv4 and IPv6 Prefix PDUs until
    /// End of Data is received.
    ///
    /// Returns `Vec<RpkiRoaRecord>` ready for database storage.
    ///
    /// # Protocol Flow
    ///
    /// 1. Connect to RTR server
    /// 2. Send Reset Query (start with v1, handle version negotiation)
    /// 3. Receive Cache Response
    /// 4. Collect IPv4/IPv6 Prefix PDUs (announcements only)
    /// 5. Wait for End of Data PDU
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Connection to the RTR server fails
    /// - The server returns an error
    /// - The server sends a Cache Reset (no data available)
    /// - Read/write operations time out
    pub fn fetch_roas(&self) -> Result<Vec<RpkiRoaRecord>> {
        info!(
            "Connecting to RTR server {}:{} (timeout: {:?})...",
            self.host, self.port, self.timeout
        );

        // Resolve hostname to socket address
        let addr = (&*self.host, self.port)
            .to_socket_addrs()
            .map_err(|e| {
                anyhow!(
                    "Failed to resolve RTR server {}:{}: {}",
                    self.host,
                    self.port,
                    e
                )
            })?
            .next()
            .ok_or_else(|| {
                anyhow!(
                    "No addresses found for RTR server {}:{}",
                    self.host,
                    self.port
                )
            })?;

        // Use connect_timeout to respect the configured timeout for connection
        let mut stream = TcpStream::connect_timeout(&addr, self.timeout).map_err(|e| {
            anyhow!(
                "Failed to connect to RTR server {}:{}: {}",
                self.host,
                self.port,
                e
            )
        })?;

        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        // Send Reset Query (start with v1, handle downgrade if needed)
        let reset_query = RtrResetQuery::new_v1();
        stream.write_all(&reset_query.encode())?;
        info!("Sent Reset Query (RTR v1)");

        let mut roas = Vec::new();
        let mut session_id: Option<u16> = None;
        #[allow(unused_assignments)]
        let mut serial: Option<u32> = None;
        let mut protocol_version = 1u8;

        // Read PDUs until End of Data
        loop {
            match read_rtr_pdu(&mut stream) {
                Ok(pdu) => match pdu {
                    RtrPdu::CacheResponse(resp) => {
                        session_id = Some(resp.session_id);
                        info!("Cache Response: session_id={}", resp.session_id);
                    }

                    RtrPdu::IPv4Prefix(p) => {
                        if p.is_announcement() {
                            roas.push(RpkiRoaRecord {
                                prefix: format!("{}/{}", p.prefix, p.prefix_length),
                                max_length: p.max_length,
                                origin_asn: p.asn.into(),
                                ta: String::new(), // RTR doesn't provide Trust Anchor info
                            });
                        }
                    }

                    RtrPdu::IPv6Prefix(p) => {
                        if p.is_announcement() {
                            roas.push(RpkiRoaRecord {
                                prefix: format!("{}/{}", p.prefix, p.prefix_length),
                                max_length: p.max_length,
                                origin_asn: p.asn.into(),
                                ta: String::new(), // RTR doesn't provide Trust Anchor info
                            });
                        }
                    }

                    RtrPdu::RouterKey(_) => {
                        // BGPsec router keys - skip for ROV purposes
                    }

                    RtrPdu::EndOfData(eod) => {
                        serial = Some(eod.serial_number);
                        info!(
                            "End of Data: serial={}, collected {} ROAs",
                            eod.serial_number,
                            roas.len()
                        );
                        if let (Some(refresh), Some(retry), Some(expire)) = (
                            eod.refresh_interval,
                            eod.retry_interval,
                            eod.expire_interval,
                        ) {
                            info!(
                                "Server timing parameters: refresh={}s, retry={}s, expire={}s",
                                refresh, retry, expire
                            );
                        }
                        break;
                    }

                    RtrPdu::CacheReset(_) => {
                        return Err(anyhow!(
                            "RTR server sent Cache Reset - server has no data available"
                        ));
                    }

                    RtrPdu::ErrorReport(err) => {
                        // Handle version downgrade
                        if err.error_code == RtrErrorCode::UnsupportedProtocolVersion {
                            warn!("RTR server doesn't support v1, retrying with v0...");
                            let reset_v0 = RtrResetQuery::new_v0();
                            stream.write_all(&reset_v0.encode())?;
                            protocol_version = 0;
                            continue;
                        }
                        return Err(anyhow!(
                            "RTR server error: {:?} - {}",
                            err.error_code,
                            err.error_text
                        ));
                    }

                    other => {
                        warn!("Unexpected RTR PDU type: {:?}", other);
                    }
                },
                Err(e) => {
                    return Err(anyhow!("Error reading RTR PDU: {:?}", e));
                }
            }
        }

        info!(
            "RTR fetch complete: session_id={:?}, serial={:?}, protocol_version=v{}, {} ROAs",
            session_id,
            serial,
            protocol_version,
            roas.len()
        );

        Ok(roas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtr_client_creation() {
        let client = RtrClient::new("rtr.example.com".to_string(), 8282, Duration::from_secs(60));

        assert_eq!(client.host, "rtr.example.com");
        assert_eq!(client.port, 8282);
        assert_eq!(client.timeout, Duration::from_secs(60));
    }

    // Integration test - requires a running RTR server
    // Run with: cargo test --features lens-bgpkit rtr_integration -- --ignored
    #[test]
    #[ignore]
    fn test_rtr_fetch_cloudflare() {
        let client = RtrClient::new(
            "rtr.rpki.cloudflare.com".to_string(),
            8282,
            Duration::from_secs(120),
        );

        let roas = client.fetch_roas().expect("Failed to fetch ROAs");

        // Cloudflare should have a significant number of ROAs
        assert!(
            roas.len() > 100_000,
            "Expected > 100k ROAs, got {}",
            roas.len()
        );

        // Verify ROA structure
        for roa in roas.iter().take(10) {
            assert!(!roa.prefix.is_empty());
            assert!(roa.max_length > 0);
            // TA is empty for RTR-sourced ROAs
            assert!(roa.ta.is_empty());
        }
    }
}
