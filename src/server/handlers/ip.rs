//! IP handlers for IP information lookup operations
//!
//! This module provides handlers for IP-related methods like `ip.lookup` and `ip.public`.

use crate::lens::ip::{IpInfo, IpLens, IpLookupArgs};
use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::Arc;

// =============================================================================
// ip.lookup
// =============================================================================

/// Parameters for ip.lookup
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IpLookupParams {
    /// IP address to look up
    #[serde(default)]
    pub ip: Option<String>,

    /// Return simplified response
    #[serde(default)]
    pub simple: Option<bool>,
}

/// Response for ip.lookup
#[derive(Debug, Clone, Serialize)]
pub struct IpLookupResponse {
    /// IP address
    pub ip: String,

    /// ASN (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asn: Option<i64>,

    /// AS name (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub as_name: Option<String>,

    /// Country (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,

    /// Prefix (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

impl From<IpInfo> for IpLookupResponse {
    fn from(info: IpInfo) -> Self {
        Self {
            ip: info.ip,
            asn: info.asn.as_ref().map(|a| a.asn),
            as_name: info.asn.as_ref().map(|a| a.name.clone()),
            country: info.country,
            prefix: info.asn.as_ref().map(|a| a.prefix.to_string()),
        }
    }
}

/// Handler for ip.lookup method
pub struct IpLookupHandler;

#[async_trait]
impl WsMethod for IpLookupHandler {
    const METHOD: &'static str = "ip.lookup";
    const IS_STREAMING: bool = false;

    type Params = IpLookupParams;

    fn validate(params: &Self::Params) -> WsResult<()> {
        // If IP is provided, validate it can be parsed
        if let Some(ref ip_str) = params.ip {
            ip_str
                .parse::<IpAddr>()
                .map_err(|_| WsError::invalid_params(format!("Invalid IP address: {}", ip_str)))?;
        }
        Ok(())
    }

    async fn handle(
        _ctx: Arc<WsContext>,
        _req: WsRequest,
        params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Create the IP lens
        let lens = IpLens::new();

        // Create args from params
        let args = if let Some(ref ip_str) = params.ip {
            let ip: IpAddr = ip_str
                .parse()
                .map_err(|_| WsError::invalid_params(format!("Invalid IP address: {}", ip_str)))?;
            IpLookupArgs::new(ip).with_simple(params.simple.unwrap_or(false))
        } else {
            IpLookupArgs::public_ip().with_simple(params.simple.unwrap_or(false))
        };

        // Perform the lookup
        let info = lens
            .lookup(&args)
            .map_err(|e| WsError::operation_failed(e.to_string()))?;

        // Convert to response
        let response: IpLookupResponse = info.into();
        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// ip.public
// =============================================================================

/// Parameters for ip.public (empty)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct IpPublicParams {}

/// Handler for ip.public method
pub struct IpPublicHandler;

#[async_trait]
impl WsMethod for IpPublicHandler {
    const METHOD: &'static str = "ip.public";
    const IS_STREAMING: bool = false;

    type Params = IpPublicParams;

    async fn handle(
        _ctx: Arc<WsContext>,
        _req: WsRequest,
        _params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        // Create the IP lens
        let lens = IpLens::new();

        // Look up public IP
        let args = IpLookupArgs::public_ip();
        let info = lens
            .lookup(&args)
            .map_err(|e| WsError::operation_failed(e.to_string()))?;

        // Convert to response
        let response: IpLookupResponse = info.into();
        sink.send_result(response)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;

        Ok(())
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_lookup_params_default() {
        let params = IpLookupParams::default();
        assert!(params.ip.is_none());
        assert!(params.simple.is_none());
    }

    #[test]
    fn test_ip_lookup_params_deserialization() {
        let json = r#"{"ip": "1.1.1.1"}"#;
        let params: IpLookupParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.ip, Some("1.1.1.1".to_string()));
        assert!(params.simple.is_none());

        let json = r#"{"ip": "8.8.8.8", "simple": true}"#;
        let params: IpLookupParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.ip, Some("8.8.8.8".to_string()));
        assert_eq!(params.simple, Some(true));
    }

    #[test]
    fn test_ip_lookup_params_validation() {
        // Empty params should pass (uses public IP)
        let params = IpLookupParams::default();
        assert!(IpLookupHandler::validate(&params).is_ok());

        // Valid IP should pass
        let params = IpLookupParams {
            ip: Some("1.1.1.1".to_string()),
            simple: None,
        };
        assert!(IpLookupHandler::validate(&params).is_ok());

        // Invalid IP should fail
        let params = IpLookupParams {
            ip: Some("not-an-ip".to_string()),
            simple: None,
        };
        assert!(IpLookupHandler::validate(&params).is_err());
    }

    #[test]
    fn test_ip_lookup_response_serialization() {
        let response = IpLookupResponse {
            ip: "1.1.1.1".to_string(),
            asn: Some(13335),
            as_name: Some("CLOUDFLARENET".to_string()),
            country: Some("US".to_string()),
            prefix: Some("1.1.1.0/24".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"ip\":\"1.1.1.1\""));
        assert!(json.contains("\"asn\":13335"));
        assert!(json.contains("CLOUDFLARENET"));
    }

    #[test]
    fn test_ip_public_params_default() {
        let params = IpPublicParams::default();
        let json = serde_json::to_string(&params).unwrap();
        assert_eq!(json, "{}");
    }
}
