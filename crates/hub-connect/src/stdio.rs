use std::process::Stdio;

use rmcp::model::{
    CallToolRequestParams, CallToolResult, JsonObject, ListToolsResult, PaginatedRequestParams,
};
use rmcp::service::{RunningService, ServiceError};
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, serve_client};
use serde_json::Value;
use tokio::process::Command;

use crate::ConnectError;

pub struct StdioMcpClient {
    service: RunningService<RoleClient, ()>,
}

impl StdioMcpClient {
    pub async fn connect(
        command: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<Self, ConnectError> {
        let mut cmd = Command::new(command);
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let transport =
            TokioChildProcess::new(cmd).map_err(|e| ConnectError::Spawn(e.to_string()))?;
        let service = serve_client((), transport)
            .await
            .map_err(|e| ConnectError::Handshake(e.to_string()))?;
        Ok(Self { service })
    }

    pub async fn list_tools(&self) -> Result<ListToolsResult, ConnectError> {
        self.service
            .peer()
            .list_tools(Some(PaginatedRequestParams::default()))
            .await
            .map_err(map_service_err)
    }

    pub async fn call_tool(&self, name: &str, args: Value) -> Result<CallToolResult, ConnectError> {
        let arguments: JsonObject = args.as_object().cloned().unwrap_or_default();
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments);
        self.service
            .call_tool(params)
            .await
            .map_err(map_service_err)
    }

    pub async fn shutdown(self) -> Result<(), ConnectError> {
        self.service
            .cancel()
            .await
            .map_err(|e| ConnectError::Handshake(e.to_string()))?;
        Ok(())
    }
}

fn map_service_err(err: ServiceError) -> ConnectError {
    ConnectError::Tool(err.to_string())
}
