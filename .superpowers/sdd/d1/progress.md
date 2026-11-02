# SDD ledger — plan: docs/superpowers/plans/2026-11-02-d1-shared-permissions-lazy.md
Setup: Ruling: no git repo — no worktree/commits; checkpoints = fmt+clippy+test — cost if wrong: no per-task rollback points
Pre-flight: T2 consumes T1 Effect/PolicyEngine (ok); T3 grant_for returns T2 Grant (ok, Grant lives in editor.rs, re-exported); T4 PolicyCache used by T5 (ok); T6 consumes T2+T3 (ok); T7 consumes T2 (ok)
Task 1: complete (tests: check.ps1 → clippy 0, test 0, 89 passed; hub-policy 17/17 incl. 10 new)
Task 2: complete (tests: check.ps1  hub-policy 24/24, 7 editor tests; toml_edit 0.23)
Task 3: complete (tests: check.ps1  hub-policy 27/27, 3 new approve tests)
Task 4: Ruling: added PolicyEngine::empty() (not in plan) — avoids unreachable!() in non-test code per Global Constraints — cost if wrong: one extra pub fn
Task 4: Ruling: gate passes Some(target) to evaluate_meta_tool so connect is checked per server (was None) — required by spec §2.3 — cost if wrong: none
Task 4: complete (tests: check.ps1 → 100 passed; reload_keeps_last_good_policy RED→GREEN)
Task 5: Ruling: default-deny agent z still sees [shared] skills (plan test said results == []) — spec §2.1 says shared goes to every agent — cost if wrong: one assertion
Task 5: Ruling: skills_search drops the old "local" key; new shape {results, requestable, registry_suggestions?} — spec §2.3 — cost if wrong: agents/tools parsing "local" break (none found in repo)
Task 5: Ruling: mcp_list tool_count = null until D2 cache exists — avoids spawning servers just to count — cost if wrong: less info for agent
Task 5: complete (tests: mcp_lazy 6/6 RED→GREEN; check.ps1 → 106 passed)
Task 6: Ruling: apply_approval lives in hub-policy (plan: hub-cli) — hub-cli has no lib target so its tests can't call fns; CLI is a thin wrapper, e2e test drives the binary — cost if wrong: none
Task 6: Ruling: --no = deny this time (no policy write); --deny = deny + remember — spec §2.4 — cost if wrong: flag semantics
Task 6: complete (tests: hub-policy apply 5/5, cli_grants 5/5 RED→GREEN; check.ps1 → 116 passed)
Task 7: Ruling: resources_used_by excludes tool='mcp_list' (logged as kind connect with query as target) — otherwise search words become fake shared servers — cost if wrong: none
Task 7: complete (tests: hub-log used 1/1, agent_lazy 2/2 RED→GREEN; check.ps1 → 119 passed)
Task 8: Ruling: fixed pre-existing bug found during manual verification — install_binary failed "Access is denied" when a previous mali_gate.exe.old was still running; swap_in() tries .old, .old1… — swap_in_works_when_previous_old_copy_is_stuck RED→GREEN — cost if wrong: none
Task 8: complete (manual: init → 5 clients ✓ connected (8 tools), allow-default warning shown; agent lazy claude-code --dry-run lists agent-skills-overview, writes nothing; check.ps1 → 121 passed)
Final review: fresh reviewer (opus subagent) — 1 Critical, 7 Important, 8 Minor
Final: fixed #1 wildcard/unknown target → shared grant — grant_for_refuses_wildcards + wildcard_request_never_becomes_a_grant RED→GREEN; gate denies unknown skill/server before ask (unknown_or_wildcard_targets_are_denied_without_asking — RED not separately observed: pre-fix it blocks 120s on ask), suite 131/131
Final: fixed #2 agent mode=deny ignored by evaluate_mcp_tool — agent_mode_deny_beats_shared_and_allow_tools RED→GREEN, suite 131/131
Final: fixed #3 invalid policy at startup failed open — broken_policy_at_startup_is_an_error RED→GREEN (also keeps last good when file vanishes, poisoned lock, transient read errors = minor #11), suite 131/131
Final: fixed #4 agent lazy kept allow ["*"]; warning missed it — agent_lazy_strips_allow_star + sees_everything_detects_allow_default_and_star + remove_agent_wildcards_strips_star RED→GREEN; sample policy no longer ships allow ["*"], suite 131/131
Final: fixed #5 revoke deleted deny_tools — revoke_agent_server_keeps_deny_tools RED→GREEN, suite 131/131
Final: fixed #6 approval race (policy written then "already resolved") — resolve first, then write — request_is_claimed_before_policy_write RED→GREEN, suite 131/131
Final: Ruling: revoke never removes a deny (revoke_and_deny test updated) — revoke = take back a grant, not un-deny — cost if wrong: need a separate "undeny" in D5
Final: Ruling: #7 keep --shared as default and --yes alias — user approved "ให้ทุก agent (ค่าเริ่มต้น)" in design §1 — cost if wrong: an accidental Enter widens access to all agents
Final: Ruling: #8 not a regression — old evaluate_skill also used agents.get(agent_id) without "*" fallback for rules — cost if wrong: "*" rule tables stay ignored (only default inherited)
Final: minor (deferred): #9 --no and --deny both stored as grant_scope "deny"; no `approval` event logged (spec §2.4.3)
Final: minor (deferred): #10 PolicyEditor::save — no lock / fixed tmp name / no fsync (concurrent writers lose updates)
Final: minor (deferred): #12 grant to new agent creates default "ask" even if "*" default is deny
Final: minor (deferred): #13 --shared on already-shared server with restrictive shared allow_tools → asked again
Final: minor (deferred): #14 init appends raw text to policy.toml instead of PolicyEditor; child() uses unreachable!
Final: minor (deferred): #15 mcp_list logged as kind=connect (query as target) — pollutes stats/TUI
Final: minor (deferred): #16 swap_in leaves no binary if final rename fails
