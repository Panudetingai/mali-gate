//! trait `Connector` + การเชื่อม MCP stdio (spawn / kill, จัดการ process ค้าง),
//! MCP HTTP และ REST (OpenAPI → tool) — lazy connect / release

mod config;
mod manager;
mod stdio;

pub use config::{HubConfig, McpServerSpec};
pub use manager::{ConnectorManager, block_on};
pub use stdio::StdioMcpClient;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConnectError {
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("mcp handshake failed: {0}")]
    Handshake(String),
    #[error("not connected: {0}")]
    NotConnected(String),
    #[error("tool call failed: {0}")]
    Tool(String),
    #[error("config: {0}")]
    Config(String),
    #[error("unsupported transport: {0}")]
    Unsupported(String),
}
