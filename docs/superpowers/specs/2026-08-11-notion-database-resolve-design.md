# Notion Database Resolution Design

## Problem

Latte currently binds Notion databases primarily by global title matching. Initialization can select an unrelated database when multiple databases share a title. Full pull/reset then reuses the stored IDs without validating them, so a stale or incorrect configuration continues pulling the wrong database.

The resolution rules must be shared by setup, pull/reset, lazy database creation, and incremental sync. Ambiguous matches must never be selected silently.

## Goals

- Resolve each Latte database against the configured Notion root page and its nested pages.
- Validate an existing database ID before using it.
- Automatically repair a stale ID when exactly one valid candidate exists.
- Surface multiple valid candidates for explicit user selection.
- Persist the selected database IDs so later syncs use the corrected mapping.
- Make reset-and-pull resolve the mapping before replacing local data.
- Preserve automatic creation when a required database is genuinely missing during setup.

## Non-goals

- Migrating or renaming existing Notion databases.
- Deleting duplicate databases automatically.
- Changing the local SQLite schema or Notion page property formats.
- Selecting among arbitrary global databases based only on title or fuzzy similarity.

## Database Identity

The supported database kinds and required property checks are:

| Kind | Title | Required properties |
|---|---|---|
| `events` | `时间碎片` | `开始`, `结束`, `内容`, `标签` with select type |
| `expenses` | `金钱记录` | `金额`, `时间`, `分类` with select type |
| `projects` | `项目管理` | `状态`, `开始`, `截止`, `备注` |
| `notes` | `📚 知识库` | `父级` with relation type |
| `ideas` | `好想法` | existing idea schema |
| `tasks` | `今日任务` | existing task schema |
| `daily` | `✅ 每日打卡` | existing daily schema |

The title property itself is required according to the existing property builders. The resolver must use the actual Notion database property names and types, not only the display title.

A candidate is valid only when:

1. Its database object is accessible to the integration.
2. Its title exactly matches the expected title for the kind.
3. Its parent is the configured root page or a descendant reachable from that root.
4. Its required property schema matches the kind.

## Resolution Algorithm

Expose one resolver used by all flows. It accepts the current `Config`, a set of required kinds, and a mode indicating whether missing databases may be created.

For each kind:

1. If the configured ID exists, fetch the database and validate title, parent scope, and schema.
2. If it passes, keep it without user interaction.
3. If it fails, enumerate accessible databases under the configured root page tree and filter by exact title and schema.
4. If exactly one candidate remains, select it and mark the configuration as repaired.
5. If multiple candidates remain, return an ambiguity result containing each candidate's ID, title, parent path, and schema status. Do not select one.
6. If no candidate remains:
   - setup mode may create the database under the resolved root page;
   - pull/reset mode returns a missing-database result and does not guess globally.
7. Persist repaired or manually selected IDs atomically after the complete mapping is valid.

Global search may remain as diagnostic information only. It must not be used as an automatic fallback for pull/reset because it can select an unrelated same-title database.

## Backend API

Add a database resolution API that returns per-kind state:

- `POST /api/databases/resolve`
- Request: optional `{ "force": true }`. `force` skips accepting an existing valid ID and re-enumerates candidates.
- Response contains:
  - `ok`: all requested required kinds are uniquely resolved;
  - `bindings`: current kind, title, ID, parent path, and schema status;
  - `ambiguous`: kinds with multiple candidates;
  - `missing`: kinds with no valid candidate;
  - `candidates`: candidate IDs and display metadata for ambiguous kinds.

Add a selection API:

- `POST /api/databases/select`
- Request: `{ "kind": "events", "database_id": "..." }`.
- Validate the selected ID using the same resolver rules before persisting it.
- Reject IDs outside the configured root tree or with an invalid schema.

Use the resolver internally in:

- `POST /api/setup` before saving the final configuration;
- `POST /api/sync/pull` before calling `pull_all`;
- reset flow through `sync/pull`;
- lazy creation and incremental sync whenever a configured optional database ID is missing or invalid.

When pull/reset encounters ambiguity or missing databases, return a conflict response with structured JSON rather than starting a partial replacement.

## Frontend Behavior

The setup view and settings/status area consume the resolution response:

- Show a binding summary for each database kind.
- For an ambiguous kind, show candidates with database title, ID suffix, parent path, and schema status.
- Require one explicit selection before retrying setup or pull.
- For a missing required database, show the exact expected title and why it was not accepted.
- On reset, resolve first; only clear local data after resolution succeeds. This prevents losing local data when Notion mapping cannot be established.
- After selection or automatic repair, refresh status and panel data.

No silent first-match behavior remains.

## Error and Data Safety Rules

- Existing local data must not be deleted before database resolution succeeds.
- A failed or ambiguous resolution leaves the current configuration and local data unchanged.
- A resolver network/API failure is reported separately from a missing database.
- Configuration writes are atomic and preserve unrelated AI, reminder, shortcut, and external API settings.
- A manually selected ID is revalidated on every future forced resolution.

## Testing

Backend unit tests must cover:

- Exact title and required-property matching.
- Same-title candidates where one is outside the configured root.
- Multiple valid candidates returning ambiguity rather than selecting one.
- Stale configured ID repaired by one valid in-tree candidate.
- Invalid schema rejected even when title matches.
- Missing database behavior differs between setup and pull/reset modes.
- Reset does not clear local data when resolution fails.
- Selected database IDs are persisted and reused by the next pull.

API integration tests must cover resolve, select, and pull conflict responses. Existing setup, pull, and sync tests must continue to pass.

Frontend verification must cover the candidate selector state, the no-data-loss reset ordering, and successful retry after selection.

## Acceptance Criteria

- Initialization never silently binds a same-title database outside the configured root tree.
- Reset-and-pull revalidates and repairs stale database IDs before replacing local data.
- Multiple valid candidates require explicit user selection.
- A valid existing mapping does not prompt the user on every normal sync.
- A selected mapping survives restart and is used by subsequent pull and sync operations.
- All existing Notion synchronization behavior remains compatible with the current property schemas.
