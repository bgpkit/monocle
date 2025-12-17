//! System handlers for introspection methods
//!
//! This module provides handlers for system-level methods like `system.info`
//! which allow clients to discover server capabilities.

use crate::server::handler::{WsContext, WsError, WsMethod, WsRequest, WsResult};
use crate::server::op_sink::WsOpSink;
use crate::server::protocol::SystemInfo;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// =============================================================================
// system.info
// =============================================================================

/// Parameters for system.info (empty)
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SystemInfoParams {}

/// Handler for system.info method
pub struct SystemInfoHandler;

#[async_trait]
impl WsMethod for SystemInfoHandler {
    const METHOD: &'static str = "system.info";
    const IS_STREAMING: bool = false;

    type Params = SystemInfoParams;

    async fn handle(
        _ctx: Arc<WsContext>,
        _req: WsRequest,
        _params: Self::Params,
        sink: WsOpSink,
    ) -> WsResult<()> {
        let info = SystemInfo::default();
        sink.send_result(info)
            .await
            .map_err(|e| WsError::internal(e.to_string()))?;
        Ok(())
    }
}

// =============================================================================
// system.methods (optional)
// =============================================================================

/// Parameters for system.methods
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SystemMethodsParams {}

/// Method info for system.methods response
#[derive(Debug, Clone, Serialize)]
pub struct MethodInfo {
    /// Method name
    pub name: &'static str,
    /// Whether the method is streaming
    pub streaming: bool,
}

/// Response for system.methods
#[derive(Debug, Clone, Serialize)]
pub struct SystemMethodsResponse {
    /// List of available methods
    pub methods: Vec<MethodInfo>,
}

/// Handler for system.methods
///
/// This handler needs to be provided with the list of methods at construction time.
pub struct SystemMethodsHandler {
    methods: Vec<MethodInfo>,
}

impl SystemMethodsHandler {
    /// Create a new handler with the given method list
    pub fn new(methods: Vec<MethodInfo>) -> Self {
        Self { methods }
    }

    /// Expose the stored method list (used by router integration).
    pub fn methods(&self) -> &[MethodInfo] {
        &self.methods
    }

    /// Create method info list from method names and streaming flags
    pub fn from_method_names(methods: Vec<(&'static str, bool)>) -> Self {
        let methods = methods
            .into_iter()
            .map(|(name, streaming)| MethodInfo { name, streaming })
            .collect();
        Self { methods }
    }
}

// Note: SystemMethodsHandler can't implement WsMethod trait directly because
// it needs state (the method list). Instead, it's used differently in the router.

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_info_params_default() {
        let params = SystemInfoParams::default();
        let json = serde_json::to_string(&params).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn test_system_info_default() {
        let info = SystemInfo::default();
        assert_eq!(info.protocol_version, 1);
        assert!(info.features.streaming);
        assert!(!info.features.auth_required);
    }

    #[test]
    fn test_method_info_serialization() {
        let info = MethodInfo {
            name: "time.parse",
            streaming: false,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"time.parse\""));
        assert!(json.contains("\"streaming\":false"));
    }

    #[test]
    fn test_system_methods_handler_creation() {
        let handler = SystemMethodsHandler::from_method_names(vec![
            ("time.parse", false),
            ("parse.start", true),
        ]);
        assert_eq!(handler.methods.len(), 2);
        assert_eq!(handler.methods[0].name, "time.parse");
        assert!(!handler.methods[0].streaming);
        assert!(handler.methods[1].streaming);
    }
}
