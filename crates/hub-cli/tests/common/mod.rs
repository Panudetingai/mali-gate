//! ตัวช่วย integration test: สร้าง MALI_GATE_HOME ชั่วคราว + คุย MCP กับ `mali_gate serve` จริงผ่าน stdio
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};

/// home ชั่วคราวที่ลบตัวเองเมื่อจบเทสต์
pub struct TempHome(pub PathBuf);

impl TempHome {
    pub fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("mg-it-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// คัดลอก fixture skills จาก `<repo>/tests/fixtures/<name>`
    pub fn add_fixture_skills(&self, names: &[&str]) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        for n in names {
            copy_dir(&fixtures.join(n), &self.0.join("skills").join(n));
        }
    }

    pub fn write(&self, rel: &str, text: &str) {
        std::fs::write(self.0.join(rel), text).unwrap();
    }

    pub fn db(&self) -> PathBuf {
        self.0.join("mali_gate.db")
    }

    /// ลงทะเบียน agent แล้วคืน token
    pub fn add_agent(&self, id: &str) -> String {
        let store = hub_policy::AgentStore::open(&self.db()).unwrap();
        store.add(id, id).unwrap().0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// client MCP แบบ JSON-RPC บรรทัดละข้อความ
pub struct McpClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    next_id: i64,
    pub init: Value,
}

impl McpClient {
    pub fn spawn(home: &TempHome, token: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_mali_gate"))
            .arg("serve")
            .env("MALI_GATE_HOME", home.path())
            .env("MALI_GATE_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
        });
        let mut c = Self {
            child,
            stdin,
            rx,
            next_id: 1,
            init: Value::Null,
        };
        c.init = c.request(
            "initialize",
            json!({"protocolVersion":"2025-06-18","capabilities":{},
                   "clientInfo":{"name":"it","version":"1"}}),
        );
        c.notify("notifications/initialized");
        c
    }

    fn notify(&mut self, method: &str) {
        writeln!(self.stdin, "{}", json!({"jsonrpc":"2.0","method":method})).unwrap();
        self.stdin.flush().unwrap();
    }

    /// ส่ง request แล้วคืน `result` (panic ถ้าได้ error หรือหมดเวลา)
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let v = self
                .rx
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("no response to {method}"));
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                if let Some(err) = v.get("error") {
                    panic!("{method} error: {err}");
                }
                return v["result"].clone();
            }
        }
    }

    pub fn tool_names(&mut self) -> Vec<String> {
        let r = self.request("tools/list", json!({}));
        r["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect()
    }

    /// เรียก tool แล้วคืน (ข้อความ, isError)
    pub fn call_raw(&mut self, name: &str, args: Value) -> (String, bool) {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        let text = r["content"][0]["text"].as_str().unwrap_or("").to_string();
        (text, r["isError"].as_bool().unwrap_or(false))
    }

    /// เรียก tool ที่คาดว่าสำเร็จและคืน JSON
    pub fn call_json(&mut self, name: &str, args: Value) -> Value {
        let (text, is_err) = self.call_raw(name, args);
        assert!(!is_err, "{name} failed: {text}");
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("{name} not JSON: {text}"))
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}
