# Notion Database Resolution Enhancement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent initialization and reset/pull from silently binding or using the wrong Notion database by validating stored IDs, resolving only in the configured root tree, and requiring explicit selection for ambiguity.

**Architecture:** Extend `NotionClient` with one database-kind-aware resolver that validates title, parent scope, and required property schema. `setup`, full pull/reset, lazy optional database creation, and incremental sync all use the resolver. Add structured resolve/select APIs and a frontend selector/status surface; reset resolves before deleting local data.

**Tech Stack:** Rust 2024, Axum, reqwest, serde_json, SQLite, Vue 3, Element Plus, existing Notion API client.

---

### Task 1: Define database kinds, candidates, and resolver result types

**Files:**
- Modify: `src/notion.rs` near `DatabaseIds` and `SetupVerifyResult`
- Test: `src/notion.rs` unit-test module

- [ ] **Step 1: Write failing tests for exact kind/title/schema classification**

Add tests using representative Notion database JSON for `events`, `expenses`, `projects`, `notes`, `ideas`, `tasks`, and `daily`. Assert that exact titles with required property types are accepted, same titles with missing/wrong properties are rejected, and unknown titles are ignored.

- [ ] **Step 2: Run the focused tests and verify failure**

Run `cargo test notion::tests -- --nocapture`. Expected: the new tests fail because the kind-aware classifier does not exist.

- [ ] **Step 3: Add `DatabaseKind`, candidate metadata, and resolution status types**

Define stable serialized kinds (`events`, `expenses`, `projects`, `notes`, `ideas`, `tasks`, `daily`), expected titles, candidate metadata (`id`, `title`, `parent_path`, `schema_ok`), and per-kind resolution states (`Resolved`, `Ambiguous`, `Missing`, `Invalid`). Keep `DatabaseIds` compatible with existing required fields and optional config IDs.

- [ ] **Step 4: Implement pure title/schema classification**

Add a pure helper that accepts a Notion database object and `DatabaseKind`, checks exact title and required property names/types, and returns candidate metadata or rejection reason. Do not use fuzzy matching.

- [ ] **Step 5: Run focused tests and commit**

Run `cargo test notion::tests -- --nocapture`. Expected: all classifier tests pass. Commit as `test: cover notion database classification` followed by the implementation if the repository convention requires separate commits.

### Task 2: Implement root-tree discovery and stored-ID validation

**Files:**
- Modify: `src/notion.rs` around `child_objects`, `resolve_root`, `find_latte_databases`, and `setup_databases`
- Test: `src/notion.rs` unit-test module

- [ ] **Step 1: Write failing tests for scope and ambiguity**

Test that a matching database outside the configured root is excluded, one valid in-tree candidate auto-resolves, two valid in-tree candidates return ambiguity, and an invalid stored ID falls back to discovery.

- [ ] **Step 2: Run focused tests and verify failure**

Run `cargo test notion::tests -- --nocapture`. Expected: scope/ambiguity tests fail before discovery helpers exist.

- [ ] **Step 3: Add paginated root-tree enumeration**

Implement a Notion client helper that recursively traverses page/database children below `parent_page_id`, preserving parent paths and pagination. It must not use global `/search` as an automatic pull fallback.

- [ ] **Step 4: Add stored-ID validation**

Fetch `/databases/{id}` and validate exact kind title, required schema, and parent ancestry against the configured root. Return structured invalid reasons rather than silently accepting an accessible but unrelated database.

- [ ] **Step 5: Add unique/ambiguous/missing resolution**

For each requested kind: accept a valid configured ID; otherwise filter root-tree candidates by exact title/schema; return one candidate, ambiguity with all candidates, or missing. Keep setup creation available only in setup mode.

- [ ] **Step 6: Run focused tests and commit**

Run `cargo test notion::tests -- --nocapture`. Expected: all discovery and validation tests pass. Commit as `feat: resolve notion databases within configured root`.

### Task 3: Integrate resolver with setup, pull/reset, and sync

**Files:**
- Modify: `src/api.rs` setup/pull handlers and route registration
- Modify: `src/sync.rs` `ensure_db_id` and `pull_incremental`
- Modify: `src/config.rs` only if atomic binding persistence requires a helper
- Test: `src/api.rs` integration tests and existing sync tests

- [ ] **Step 1: Write failing API tests**

Add tests for `POST /api/databases/resolve`, `POST /api/databases/select`, ambiguous resolution returning conflict, invalid selection returning conflict, and pull failing before local replacement when resolution is unresolved.

- [ ] **Step 2: Add resolver APIs and structured response JSON**

Register protected routes. `resolve` accepts optional `force`; `select` accepts `{kind,database_id}`. Both return binding state, candidates, missing kinds, and ambiguity. Selection revalidates before persisting. Persist all changed IDs together while preserving unrelated config fields.

- [ ] **Step 3: Resolve before `sync_pull` and before reset deletion**

Make `sync_pull` resolve the required and configured optional kinds first. Update config with automatic repairs. Return `409` with structured resolution data on ambiguity/missing. Ensure the frontend reset sequence resolves before calling `reset_data`; if resolution fails, local data remains untouched.

- [ ] **Step 4: Replace global lazy lookup and stale incremental IDs**

Update `ensure_db_id` to validate configured IDs and use root-scoped resolution before creating optional databases. Update `pull_incremental` to skip unresolved kinds and record a structured sync error rather than querying a stale unrelated ID.

- [ ] **Step 5: Run backend tests and commit**

Run `cargo test api::tests sync::tests -- --nocapture`. Expected: new resolution/conflict tests and existing synchronization tests pass. Commit as `feat: integrate notion database resolution`.

### Task 4: Add frontend candidate selection and binding diagnostics

**Files:**
- Modify: `frontend/src/api.js`
- Modify: `frontend/src/views/SetupView.vue`
- Modify: `frontend/src/App.vue`
- Test/verify: browser flow against a configured test instance

- [ ] **Step 1: Add API wrappers**

Add `resolveDatabases(force)` and `selectDatabase(kind, databaseId)` wrappers with the existing `unwrap` error convention.

- [ ] **Step 2: Add setup ambiguity selector**

Render candidate rows with kind, title, ID suffix, parent path, and schema status. Require one candidate per ambiguous kind before retrying setup/pull. Display missing and invalid reasons without silently choosing.

- [ ] **Step 3: Add binding diagnostics and safe reset flow**

Expose current bindings and a force-resolve action in the settings/status surface. Change reset to resolve first, then confirm/delete local data only after resolution succeeds. Refresh panels/status after selection or successful pull.

- [ ] **Step 4: Build and browser-smoke the flows**

Run `cd frontend && pnpm build`. In the browser, verify one valid binding proceeds without a prompt, ambiguity renders candidates, selecting a candidate retries successfully, and unresolved reset leaves local data intact.

- [ ] **Step 5: Commit frontend integration**

Commit as `feat: add notion database binding diagnostics`.

### Task 5: Full verification and final commit

**Files:**
- Verify: all changed files
- Update: `README.md` setup/sync behavior if API or user workflow changed

- [ ] **Step 1: Run the complete backend suite**

Run `cargo test`. Expected: all tests pass.

- [ ] **Step 2: Run frontend build**

Run `cd frontend && pnpm build`. Expected: build succeeds with only existing chunk-size warnings if present.

- [ ] **Step 3: Review the final diff and documentation**

Confirm no global title-only automatic fallback remains in pull/reset paths, no local data is deleted before resolution, and README documents ambiguous database selection and binding repair.

- [ ] **Step 4: Commit final documentation/verification changes**

Commit as `docs: document notion database rebinding behavior` if documentation changed. Confirm `git status` is clean and report test evidence.
