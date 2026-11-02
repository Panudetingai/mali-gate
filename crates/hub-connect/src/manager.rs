use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use rmcp::model::CallToolResult;
use serde_json::Value;
use tokio::runtime::Runtime;

use crate::ConnectError;
use crate::config::{HubConfig, McpServerSpec};
use crate::stdio::StdioMcpClient;

static CONNECT_RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("connector runtime"));

pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    CONNECT_RT.block_on(future)
}

struct LiveConn {
    stdio: StdioMcpClient,
    last_used: Instant,
}

pub struct ConnectorManager {
    config: HubConfig,
    inner: Mutex<HashMap<String, LiveConn>>,
}

impl ConnectorManager {
    pub fn new(config: HubConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// server ทั้งหมดใน config: (ชื่อ, คำอธิบาย) เรียงตามชื่อ
    pub fn servers(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = self
            .config
            .mcp
            .servers
            .iter()
            .map(|(n, s)| (n.clone(), s.description().map(str::to_string)))
            .collect();
        out.sort();
        out
    }

    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.config.hub.idle_timeout_secs.max(30))
    }

    pub fn sweep_idle(&self) {
        let timeout = self.idle_timeout();
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let stale: Vec<String> = guard
            .iter()
            .filter(|(_, c)| c.last_used.elapsed() > timeout)
            .map(|(k, _)| k.clone())
            .collect();
        for name in stale {
            if let Some(conn) = guard.remove(&name) {
                drop(guard);
                let _ = block_on(conn.stdio.shutdown());
                guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            }
        }
    }

    pub fn connect(&self, server: &str) -> Result<Vec<String>, ConnectError> {
        self.sweep_idle();
        if self
            .inner
            .lock()
            .map_err(|_| ConnectError::NotConnected("lock".into()))?
            .contains_key(server)
        {
            return self.list_cached_tools(server);
        }
        let spec = self
            .config
            .mcp
            .servers
            .get(server)
            .ok_or_else(|| ConnectError::Config(format!("unknown mcp server: {server}")))?
            .clone();
        let client = match spec {
            McpServerSpec::Stdio {
                command, args, env, ..
            } => {
                let env_pairs: Vec<(String, String)> = env.into_iter().collect();
                block_on(StdioMcpClient::connect(&command, &args, &env_pairs))?
            }
            McpServerSpec::Http { .. } => {
                return Err(ConnectError::Unsupported(
                    "http transport: enable hub-connect http feature (streamable HTTP client)"
                        .into(),
                ));
            }
            McpServerSpec::Rest { .. } => {
                return Err(ConnectError::Unsupported(
                    "rest/openapi transport not implemented yet".into(),
                ));
            }
        };
        let tools = block_on(client.list_tools())?;
        let names: Vec<String> = tools.tools.iter().map(|t| t.name.to_string()).collect();
        self.inner
            .lock()
            .map_err(|_| ConnectError::NotConnected("lock".into()))?
            .insert(
                server.to_string(),
                LiveConn {
                    stdio: client,
                    last_used: Instant::now(),
                },
            );
        Ok(names)
    }

    fn list_cached_tools(&self, server: &str) -> Result<Vec<String>, ConnectError> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| ConnectError::NotConnected("lock".into()))?;
        let conn = guard
            .get_mut(server)
            .ok_or_else(|| ConnectError::NotConnected(server.into()))?;
        conn.last_used = Instant::now();
        let tools = block_on(conn.stdio.list_tools())?;
        Ok(tools.tools.iter().map(|t| t.name.to_string()).collect())
    }

    pub fn call(
        &self,
        server: &str,
        tool: &str,
        args: Value,
    ) -> Result<CallToolResult, ConnectError> {
        self.sweep_idle();
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| ConnectError::NotConnected("lock".into()))?;
        let conn = guard
            .get_mut(server)
            .ok_or_else(|| ConnectError::NotConnected(format!("not connected: {server}")))?;
        conn.last_used = Instant::now();
        block_on(conn.stdio.call_tool(tool, args))
    }

    pub fn release(&self, server: &str) -> Result<(), ConnectError> {
        let conn = self
            .inner
            .lock()
            .map_err(|_| ConnectError::NotConnected("lock".into()))?
            .remove(server);
        if let Some(conn) = conn {
            block_on(conn.stdio.shutdown())?;
        }
        Ok(())
    }

    pub fn release_all(&self) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let keys: Vec<String> = guard.keys().cloned().collect();
        for key in keys {
            if let Some(conn) = guard.remove(&key) {
                drop(guard);
                let _ = block_on(conn.stdio.shutdown());
                guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            }
        }
    }
}

impl Drop for ConnectorManager {
    fn drop(&mut self) {
        self.release_all();
    }
}
