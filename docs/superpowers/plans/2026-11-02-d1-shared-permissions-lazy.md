# D1 — Shared Permissions + Lazy Visibility Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Agents see and load only what they are allowed; anything else they can request once, the user answers shared/agent/once/no, and the hub remembers by writing `policy.toml`.

**Architecture:** `hub-policy` gains a `[shared]` section, a new decision order, server-level MCP verdicts and a comment-preserving `PolicyEditor` (toml_edit). `hub-mcp` reloads policy on mtime change, filters `skills_search`/`connect`, adds `mcp_list`, and sets handshake instructions. The CLI `approve` command writes grants via `PolicyEditor`; `agent lazy` migrates `default = "allow"` agents.

**Tech Stack:** Rust 2024, rusqlite, toml + toml_edit, rmcp 3.8, clap.

**Spec:** `docs/superpowers/specs/2026-11-02-provisioning-control-center-design.md` (§2, §4 grant/revoke, §11 step 1)

## Global Constraints

- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass at the end of every task.
- Run cargo from **PowerShell**, not Git Bash (Git's `link.exe` shadows MSVC). If cargo cannot replace `mali_gate*.exe` (Windows lock), rename the locked file to `*.exe.locked` — never kill user processes.
- Library crates use `thiserror`; binaries use `anyhow`; no `unwrap()`/`expect()` outside tests.
- Default deny; deny beats allow; no policy = deny; tokens never printed or logged.
- Every file write to `policy.toml` is atomic (tmp + rename) and backs up once to `policy.toml.mali-gate.bak`.
- Schema changes go through an idempotent migration (`ALTER TABLE … ADD COLUMN` guarded by `PRAGMA table_info`).
- Repo is not a git repository: "Commit" steps are replaced by "Checkpoint" (run the three checks above).

## Review Focus

1. Hand-written `policy.toml` using inline tables (`skills = { allow = ["*"], deny = [] }`) and comments — the editor must update inline tables in place and keep comments (test in Task 2).
2. Granting to an agent that has no `[agents.<id>]` section — the editor must create it with `default = "ask"` (test in Task 2).
3. `policy.toml` edited to invalid TOML while `serve` runs — serve keeps the last good policy and keeps answering (test in Task 4).
4. Granting something already granted — no duplicate array entries; second call reports `changed = false` (test in Task 2).
5. `skills_search` for an agent whose default is `deny` — returns empty `results` and empty `requestable`, never skill bodies (test in Task 5).

---

### Task 1: `[shared]` section + new decision order

**Files:**
- Modify: `crates/hub-policy/src/types.rs` (add `SharedPolicy`, field `shared` on `PolicyDocument`)
- Modify: `crates/hub-policy/src/evaluate.rs` (rewrite `evaluate_skill`, `evaluate_mcp_tool`, `evaluate_meta_tool`; add `evaluate_mcp_server`)
- Test: unit tests in `crates/hub-policy/src/evaluate.rs`

**Interfaces:**
- Produces:
  - `pub struct SharedPolicy { pub skills: Vec<String>, pub mcp: Vec<String>, pub mcp_tools: HashMap<String, McpToolRules> }` (all `#[serde(default)]`), `PolicyDocument.shared: SharedPolicy` (`#[serde(default)]`)
  - `PolicyEngine::evaluate_skill(&self, agent: &str, skill: &str) -> Verdict`
  - `PolicyEngine::evaluate_mcp_tool(&self, agent: &str, server: &str, tool: &str) -> Verdict`
  - `PolicyEngine::evaluate_mcp_server(&self, agent: &str, server: &str) -> Verdict` (new)
  - `PolicyEngine::agent_default(&self, agent: &str) -> Effect` (pub; `[agents.A].default` → `[agents."*"].default` → `Deny`)
  - `evaluate_meta_tool`: `skills_search`, `mcp_list`, `release` always return `Allow` (they only expose already-filtered data, and `ask` on every search would block the agent); `connect` → `evaluate_mcp_server`; `skills_install` stays `Allow` (it has its own approval queue).

Decision order (spec §2.2), for agent A:
- skill: A.skills.deny match → Deny; A.skills.allow match or `shared.skills` match → Allow; else `agent_default(A)`. No `[agents.A]` and no `"*"` → Deny.
- mcp tool: A.mcp[server].deny_tools match → Deny; `shared.mcp_tools[server].deny_tools` match → Deny; A.mcp[server].allow_tools match → Allow; server in `shared.mcp` and (`shared.mcp_tools[server].allow_tools` empty or match) → Allow; A.mcp[server].mode set → that; else `agent_default(A)`.
- mcp server: A.mcp[server].mode == Deny → Deny; server in `shared.mcp` or A.mcp[server] has non-empty `allow_tools` or mode Allow → Allow; mode Ask → Ask; else `agent_default(A)`.

- [ ] **Step 1: Write failing tests** — one test per row, using this policy text:

```toml
[shared]
skills = ["pdf-*"]
mcp = ["github"]
[shared.mcp_tools.github]
deny_tools = ["delete_*"]

[agents.a]
default = "ask"
skills = { allow = ["deploy-docs"], deny = ["deploy-*", "pdf-secret"] }
mcp.postgres = { mode = "ask" }
mcp.slack = { allow_tools = ["post_*"] }

[agents.b]
default = "allow"

[agents."*"]
default = "deny"
```

```rust
#[test] fn shared_skill_allowed_for_everyone() { assert_eq!(e.evaluate_skill("a","pdf-merge"), Verdict::Allow); assert_eq!(e.evaluate_skill("zz","pdf-merge"), Verdict::Allow); }
#[test] fn agent_deny_beats_shared() { assert_eq!(e.evaluate_skill("a","pdf-secret"), Verdict::Deny); }
#[test] fn agent_deny_beats_agent_allow() { assert_eq!(e.evaluate_skill("a","deploy-docs"), Verdict::Deny); }
#[test] fn unlisted_skill_uses_default() { assert_eq!(e.evaluate_skill("a","x"), Verdict::Ask); assert_eq!(e.evaluate_skill("b","x"), Verdict::Allow); assert_eq!(e.evaluate_skill("zz","x"), Verdict::Deny); }
#[test] fn shared_server_tools() { assert_eq!(e.evaluate_mcp_tool("a","github","search"), Verdict::Allow); assert_eq!(e.evaluate_mcp_tool("a","github","delete_repo"), Verdict::Deny); }
#[test] fn agent_tool_allow_and_mode() { assert_eq!(e.evaluate_mcp_tool("a","slack","post_msg"), Verdict::Allow); assert_eq!(e.evaluate_mcp_tool("a","postgres","query"), Verdict::Ask); }
#[test] fn server_verdicts() { assert_eq!(e.evaluate_mcp_server("a","github"), Verdict::Allow); assert_eq!(e.evaluate_mcp_server("a","slack"), Verdict::Allow); assert_eq!(e.evaluate_mcp_server("a","postgres"), Verdict::Ask); assert_eq!(e.evaluate_mcp_server("a","other"), Verdict::Ask); assert_eq!(e.evaluate_mcp_server("zz","other"), Verdict::Deny); assert_eq!(e.evaluate_mcp_server("zz","github"), Verdict::Allow); }
#[test] fn listing_meta_tools_always_allowed() { for t in ["skills_search","mcp_list","release"] { assert_eq!(e.evaluate_meta_tool("zz", t, None), Verdict::Allow); } }
#[test] fn old_policy_without_shared_still_parses() { PolicyEngine::from_toml("[agents.cursor]\ndefault=\"allow\"\n").unwrap(); }
```

Note: `mcp.postgres = "ask"` (bare string, spec example) must also parse — add `#[serde(untagged)]` enum or custom deserialize for `McpServerPolicy` accepting either a string effect or a table. Add test `mcp_mode_shorthand_string_parses` asserting `evaluate_mcp_server(..)` = Ask for `mcp.postgres = "ask"`.

- [ ] **Step 2: Run** `cargo test -p hub-policy` — expect compile failure / FAIL.
- [ ] **Step 3: Implement** types + evaluation per the order above. Existing callers keep compiling (signatures unchanged except new fns).
- [ ] **Step 4: Run** `cargo test -p hub-policy` — PASS, including pre-existing tests (fix any whose expectations relied on `skills_search` following `default`; that behavior is intentionally changed).
- [ ] **Step 5: Checkpoint.**

---

### Task 2: `PolicyEditor` (comment-preserving writes)

**Files:**
- Create: `crates/hub-policy/src/editor.rs`
- Modify: `crates/hub-policy/src/lib.rs` (export), `crates/hub-policy/Cargo.toml` (`toml_edit = "0.23"` — use the version already compatible with `toml` 0.9 in Cargo.lock if present; otherwise latest), workspace `Cargo.toml` `[workspace.dependencies]`
- Test: unit tests in `editor.rs`

**Interfaces:**
- Produces:
  - `pub enum Grant { Skill(String), McpServer(String) }`
  - `pub struct PolicyEditor { path: PathBuf, doc: toml_edit::DocumentMut }`
  - `PolicyEditor::open(path: &Path) -> Result<Self, PolicyError>` (missing file → empty doc)
  - `grant_shared(&mut self, g: &Grant) -> bool` — append to `[shared].skills` / `[shared].mcp`
  - `grant_agent(&mut self, agent: &str, g: &Grant) -> bool` — skill → `agents.<a>.skills.allow`; server → `agents.<a>.mcp.<server> = { mode = "allow" }` (replace existing mode)
  - `revoke(&mut self, agent: Option<&str>, g: &Grant) -> bool` — `None` = remove from shared; `Some(a)` = remove from that agent's allow / mcp entry
  - `deny_agent(&mut self, agent: &str, g: &Grant) -> bool` — skill → `skills.deny`; server → `mode = "deny"`
  - `set_default(&mut self, agent: &str, effect: Effect) -> bool`
  - `save(&self) -> Result<(), PolicyError>` — validate by `PolicyEngine::from_toml(&self.doc.to_string())`, then backup-once + atomic write
  - All mutators return `true` if the document changed. Creating a missing `[agents.<a>]` sets `default = "ask"` first.
  - `PolicyError` gains `#[error("invalid policy after edit: {0}")] Invalid(String)`.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn grant_shared_creates_section_and_keeps_comments() // input "# keep me\n[agents.a]\ndefault = \"ask\"\n" → output contains "# keep me" and engine.evaluate_skill("a","s1") == Allow
#[test] fn grant_agent_updates_inline_table() // input `[agents.cursor]\nskills = { allow = ["*"], deny = [] }` + grant_agent("cursor", Skill("x")) → re-parse: allow == ["*","x"], still one `skills` key
#[test] fn grant_agent_creates_missing_agent_with_ask_default() // grant_agent("new", McpServer("gh")) → agent_default("new") == Ask, evaluate_mcp_server("new","gh") == Allow
#[test] fn duplicate_grant_is_noop() // second grant_shared returns false; "s1" appears once in to_string()
#[test] fn revoke_and_deny() // revoke(None, Skill) removes from shared; deny_agent then evaluate == Deny
#[test] fn save_is_atomic_and_backs_up_once() // tmp dir: save twice → .mali-gate.bak exists and equals the ORIGINAL content
#[test] fn set_default_changes_effect()
```

- [ ] **Step 2: Run** `cargo test -p hub-policy editor` — FAIL.
- [ ] **Step 3: Implement** with `toml_edit` (`Item::Value(Value::InlineTable)` and `Item::Table` both handled when navigating `agents.<a>.skills`).
- [ ] **Step 4: Run** `cargo test -p hub-policy` — PASS.
- [ ] **Step 5: Checkpoint.**

---

### Task 3: Approval scopes in the store

**Files:**
- Modify: `crates/hub-policy/src/approve.rs`
- Test: unit tests in `approve.rs`

**Interfaces:**
- Produces:
  - `pub enum GrantScope { Shared, Agent, Once, Deny }` with `as_str()` → `"shared"|"agent"|"once"|"deny"` and `parse`
  - migration in `ApprovalStore::open`: add column `grant_scope TEXT` if missing
  - `ApprovalStore::get(&self, id: i64) -> Result<PendingApproval, ApproveError>`
  - `ApprovalStore::resolve_scoped(&self, id: i64, scope: GrantScope) -> Result<(), ApproveError>` — sets `resolved` = `approved` for Shared/Agent/Once, `denied` for Deny, and `grant_scope`
  - existing `resolve(id, outcome)` keeps working (scope `once`/`deny`)
  - `pub fn grant_for(kind: &str, target: &str) -> Option<Grant>` — `skill_load|skill_file` → `Skill(target)`; `connect|call` → `McpServer(target)`; else `None` (call approvals grant the whole server — spec decision recorded here)

- [ ] **Step 1: Failing tests:** `migration_adds_grant_scope_to_old_table` (create old schema by hand, open, insert, resolve_scoped works); `resolve_scoped_sets_outcome` (Shared → poll_resolved == Approved; Deny → Denied); `grant_for_maps_kinds`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p hub-policy` — PASS. **Step 5: Checkpoint.**

---

### Task 4: Serve reloads policy safely

**Files:**
- Modify: `crates/hub-mcp/src/session.rs`, `crates/hub-mcp/src/gate.rs`
- Test: unit test in `session.rs`

**Interfaces:**
- Consumes: `hub_policy::PolicyEngine`, `load_policy_from_disk`
- Produces: `HubSession::policy(&self) -> Arc<PolicyEngine>` — checks `policy.toml` mtime (stored in a `Mutex<(Option<SystemTime>, Arc<PolicyEngine>)>`); on change, re-parse; on parse error keep the previous engine and `eprintln!` one warning. Field `policy` becomes private; all gate functions call `session.policy()`.
- Gate: `check_meta` for `connect` uses `evaluate_mcp_server(agent, target)`.

- [ ] **Step 1: Failing test** `reload_keeps_last_good_policy`: write policy A to a temp file, build the reloader over that path (factor the reload logic into `PolicyCache::new(path)` + `PolicyCache::get(&self) -> Arc<PolicyEngine>` so it is testable without `HubSession::open`), change file to allow `x` → `get()` sees it; overwrite with `"[[[ broken"` → `get()` still allows `x`.
- [ ] **Step 2–4:** run (FAIL) → implement `PolicyCache` in `session.rs` → run `cargo test -p hub-mcp` (PASS).
- [ ] **Step 5: Checkpoint.**

---

### Task 5: Lazy visibility in meta-tools + `mcp_list` + instructions

**Files:**
- Modify: `crates/hub-mcp/src/lib.rs`
- Modify: `crates/hub-connect/src/config.rs` (optional `description: Option<String>` on every `McpServerSpec` variant + `pub fn description(&self) -> Option<&str>`)
- Modify: `crates/hub-connect/src/manager.rs` (`pub fn server_names(&self) -> Vec<String>`; `pub fn cached_tools(&self, server) -> Option<Vec<String>>` from the existing cache, no spawn)
- Create: `crates/hub-cli/tests/mcp_lazy.rs` (integration, spawns `env!("CARGO_BIN_EXE_mali_gate") serve` with `MALI_GATE_HOME` = temp dir)
- Create: `crates/hub-cli/tests/common/mod.rs` (JSON-RPC stdio helper: `McpClient::spawn(home, token) -> McpClient`, `call_tool(name, args) -> serde_json::Value`, `list_tools() -> Vec<String>`)

**Interfaces:**
- Consumes: Task 1 evaluators, Task 4 `HubSession::policy()`
- Produces (wire format, JSON text in the tool result):
  - `skills_search` → `{ "results": [{name, description}], "requestable": [{name, description}], "registry_suggestions": [...]? }` — `results` = verdict Allow; `requestable` = verdict Ask; Deny omitted. `registry_suggestions` key only when non-empty (keep `merge_search_json` semantics on the new object).
  - `mcp_list { query?: string }` → `{ "servers": [{ name, description, tool_count, access: "allow"|"ask" }] }` — Deny omitted; `query` filters case-insensitively on name+description; `tool_count` from cache or `null`. Logged as kind `connect`? **No** — log as `skill_search`-like listing: use `EventKind::Connect` with tool `mcp_list` (no new enum variant; keeps schema unchanged).
  - `connect` → `{ "server", "tools": [allowed], "requestable_tools": [ask] }`
  - Server instructions (exact copy): `"Mali Gate hub. Find before you load: use skills_search for skills and mcp_list for MCP servers, then skills_load / connect only what the task needs, and release when done. Items under \"requestable\" can be used — the hub will ask the user once and remember."`
- Switch `#[tool_router(server_handler)]` to `#[tool_router]` + explicit `#[tool_handler] impl ServerHandler for MaliGateMcp { fn get_info(&self) -> ServerInfo { ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(INSTRUCTIONS) } }` (rmcp 3.8 `ServerConfig`/`InitializeResult::with_instructions`).

- [ ] **Step 1: Write failing integration tests** (`crates/hub-cli/tests/mcp_lazy.rs`). Setup per test: temp home with `skills/` containing fixtures `tests/fixtures/*` copied (≥3 skills), `policy.toml` below, agent added via `AgentStore::open(home/mali_gate.db).add("a","A")` and `("z","Z")`, `config.toml` with two stdio servers `alpha`/`beta` pointing at a non-existent command (no spawn needed for listing).

```toml
[shared]
skills = ["<fixture-1>"]
mcp = ["alpha"]
[agents.a]
default = "ask"
skills = { deny = ["<fixture-3>"] }
[agents.z]
default = "deny"
```

```rust
#[test] fn search_splits_allowed_and_requestable() // agent a, query matching all fixtures: results == [fixture-1]; requestable contains fixture-2; fixture-3 nowhere
#[test] fn search_for_default_deny_agent_is_empty() // agent z: results == [] and requestable == []; response text does not contain "SKILL.md" body text of any fixture
#[test] fn mcp_list_filters_by_access() // agent a: alpha access "allow", beta access "ask"; agent z: only alpha
#[test] fn handshake_has_instructions() // initialize result .instructions contains "skills_search" and "mcp_list"
#[test] fn tools_list_has_eight_meta_tools() // names == {skills_search, skills_load, skills_read_file, skills_install, mcp_list, connect, call, release}
```

- [ ] **Step 2: Run** `cargo test -p hub-cli --test mcp_lazy` — FAIL.
- [ ] **Step 3: Implement** the filtering in `skills_search` (evaluate each hit with `policy().evaluate_skill`), `mcp_list`, `connect` filtering (`evaluate_mcp_tool` per tool), instructions, config `description`.
- [ ] **Step 4: Run** `cargo test --workspace` — PASS.
- [ ] **Step 5: Checkpoint.**

---

### Task 6: CLI — scoped approvals + skill grant/revoke

**Files:**
- Modify: `crates/hub-cli/src/approve.rs`, `crates/hub-cli/src/main.rs` (clap), `crates/hub-cli/src/slash.rs` only if `/approve` exists there
- Test: `crates/hub-cli/tests/approve_scopes.rs` (calls the library functions with a temp `MALI_GATE_HOME`; set env per test with a process-wide `Mutex` guard since env is global)

**Interfaces:**
- Consumes: Task 2 `PolicyEditor`, Task 3 `GrantScope`, `grant_for`, `get`, `resolve_scoped`
- Produces:
  - clap: `approve resolve <id> [--shared|--agent|--once|--no|--deny]` (mutually exclusive group; `--yes` kept as hidden alias of `--shared`; no flag = `--shared`)
  - `pub fn apply_approval(id: i64, scope: GrantScope) -> anyhow::Result<ApprovalApplied>` where `ApprovalApplied { agent: String, grant: Option<Grant>, policy_changed: bool }` — order: (1) write policy via `PolicyEditor` for Shared/Agent/Deny when `grant_for` is Some, (2) `resolve_scoped`. Policy written **before** resolving so the waiting agent's retry sees it.
  - `skills grant <name> [--agent A]`, `skills revoke <name> [--agent A]` → `PolicyEditor` + print result line
  - `approve list` shows `kind`, and a hint line: `mali_gate approve resolve <id> --shared | --agent | --once | --no`

- [ ] **Step 1: Failing tests:** `approve_shared_writes_policy_then_resolves` (insert pending skill_load for agent a, apply Shared → policy has shared skill, `poll_resolved` == Approved); `approve_agent_for_call_grants_server_to_agent_only`; `approve_once_does_not_touch_policy` (policy bytes unchanged); `approve_deny_writes_agent_deny`.
- [ ] **Step 2–4:** run (FAIL) → implement → `cargo test --workspace` (PASS).
- [ ] **Step 5: Checkpoint.**

---

### Task 7: Migration — `agent lazy` + init defaults

**Files:**
- Modify: `crates/hub-cli/src/agent.rs`, `crates/hub-cli/src/main.rs` (clap `agent lazy <id> [--dry-run]`), `crates/hub-cli/src/init.rs` (`ensure_policy_entry` writes `default = "ask"`; report flags agents whose `agent_default` is Allow)
- Modify: `crates/hub-log/src/query.rs` — `pub fn resources_used_by(&self, agent: &str) -> Result<UsedResources, LogError>` with `UsedResources { skills: BTreeSet<String>, mcp_servers: BTreeSet<String> }` from events where decision in (allow, ask_approved) and kind in (skill_load, skill_file → target as skill; connect, call → target as server)
- Test: unit test in `query.rs`; integration `crates/hub-cli/tests/agent_lazy.rs`

**Interfaces:**
- Consumes: Task 2 `PolicyEditor::{grant_shared, set_default}`
- Produces: `pub fn run_agent_lazy(id: &str, dry_run: bool, paint: &Palette) -> anyhow::Result<LazyReport>`; `LazyReport { moved_skills: Vec<String>, moved_servers: Vec<String>, default_changed: bool }`. Moves used resources into `[shared]`, then `set_default(id, Ask)`. Dry run prints the same lists and writes nothing.
- init report line for allow-default agents (exact copy): `policy: default = "allow" (sees everything) — run \`mali_gate agent lazy <id>\` to switch to lazy`

- [ ] **Step 1: Failing tests:** `resources_used_by_collects_allowed_only` (denied events excluded); `agent_lazy_moves_used_into_shared_and_sets_ask`; `agent_lazy_dry_run_writes_nothing`.
- [ ] **Step 2–4:** run (FAIL) → implement → `cargo test --workspace` (PASS).
- [ ] **Step 5: Checkpoint.**

---

### Task 8: Manual verification + docs

**Files:**
- Modify: `CLAUDE.md` (status table: new meta-tool `mcp_list`, `[shared]`, `approve resolve --shared|…`, `agent lazy`, `skills grant/revoke`; meta-tool table row for `mcp_list`)

- [ ] **Step 1:** `mali_gate init --no-path` → all detected clients `✓ connected (8 tools)`.
- [ ] **Step 2:** `mali_gate agent lazy claude-code --dry-run` → prints the skills/servers it would move; nothing written (compare `policy.toml` mtime).
- [ ] **Step 3:** Update CLAUDE.md. **Step 4: Checkpoint** (fmt, clippy, test).

## Later plans (not in this file)

`skills remove` (spec §4) ships with D4, next to the unused report that motivates it.

D2 MCP catalog/import · D3 pause/remove/connect + `hub-clients` · D4 usage/unused · D5 TUI control center + spinner · D6 CLI spinner + docs — each gets its own plan after D1 lands.
