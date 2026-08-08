//! SQLite 本地存储：schema + CRUD（本地优先，dirty 标记软同步）。
//!
//! 同步语义：
//! - 所有本地写操作立即生效并置 `dirty = 1`
//! - 删除：从未同步过（notion_page_id 为空）的行直接物理删除；
//!   已同步过的行置 `deleted = 1, dirty = 1`，待同步任务在 Notion 端归档后再物理删除

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension, Row, params};
use uuid::Uuid;
use chrono::{Local, NaiveDate, TimeZone};

use crate::notion::PulledData;
use crate::models::{
    Category, Event, Expense, ExtRecord, Idea, IdeaTag, Note, NoteKind, Project, ProjectStatus,
    Tag, Task, TaskPriority,
};

pub struct Db {
    conn: Connection,
}

const EVENT_COLS: &str =
    "id, start_ts, end_ts, content, tag, notion_page_id, dirty, deleted, remind, task_id";
const EXPENSE_COLS: &str = "id, item, amount_cents, ts, category, notion_page_id, dirty, deleted";
const PROJECT_COLS: &str =
    "id, name, status, start_ts, deadline_ts, note, notion_page_id, dirty, deleted";
const NOTE_COLS: &str = "id, parent_id, kind, title, content_md, created_ts, updated_ts, notion_page_id, dirty, deleted";
const IDEA_COLS: &str =
    "id, content, tag, pinned, created_ts, updated_ts, notion_page_id, dirty, deleted";
const TASK_COLS: &str =
    "id, date, title, priority, important, urgent, pomodoro_count, estimated_minutes, notes, done, created_ts, updated_ts, notion_page_id, dirty, deleted, task_type, project_id, start_ts";
const EXT_RECORD_COLS: &str =
    "ns, id, title, props_json, content_md, created_ts, updated_ts, notion_page_id, dirty, deleted";

/// 拉取覆盖后的各类计数
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct PullCounts {
    pub events: usize,
    pub expenses: usize,
    pub projects: usize,
    pub ideas: usize,
    pub tasks: usize,
    pub notes: usize,
}

/// 一条全局搜索结果；kind 由查询方填充（note/task/idea/event/expense/project）
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub snippet: String,
    /// 排序用时间戳（事件开始/消费时间/更新时间等）
    pub sort_ts: i64,
}

/// 增量拉取的 upsert 结果
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpsertKind {
    Inserted,
    Updated,
    /// 本地有未推送修改，跳过（推送优先）
    SkippedDirty,
    /// 解析结果没有 notion_page_id，无法对应远端行
    NoPageId,
}

/// 截断到最多 n 个字符，超出加省略号
fn truncate_chars(text: &str, n: usize) -> String {
    let mut chars = text.chars();
    let s: String = chars.by_ref().take(n).collect();
    if chars.next().is_some() { format!("{s}…") } else { s }
}

/// 从 body 提取 kw（已小写）前后各一段作为摘要；未命中则返回开头截断。
/// 匹配位置在 lower 化文本上找，再按字符数映射回原文切片（避免 Unicode 切片 panic）。
fn make_snippet(body: &str, kw_lower: &str) -> String {
    let text = body.replace(['\n', '\r'], " ");
    let lowered = text.to_lowercase();
    let Some(byte_pos) = lowered.find(kw_lower) else {
        return truncate_chars(&text, 60);
    };
    let char_pos = lowered[..byte_pos].chars().count();
    let kw_len = kw_lower.chars().count();
    let total = text.chars().count();
    let start = char_pos.saturating_sub(20);
    let end = (char_pos + kw_len + 40).min(total);
    let window: String = text.chars().skip(start).take(end - start).collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        window,
        if end < total { "…" } else { "" }
    )
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    /// 内存数据库（仅测试用）
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self { conn };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                start_ts INTEGER NOT NULL,
                end_ts INTEGER,
                content TEXT NOT NULL DEFAULT '',
                tag TEXT NOT NULL DEFAULT '生活',
                remind INTEGER NOT NULL DEFAULT 0,
                notion_page_id TEXT,
                dirty INTEGER NOT NULL DEFAULT 1,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS expenses (
                id TEXT PRIMARY KEY,
                item TEXT NOT NULL DEFAULT '',
                amount_cents INTEGER NOT NULL DEFAULT 0,
                ts INTEGER NOT NULL,
                category TEXT NOT NULL DEFAULT '其他',
                notion_page_id TEXT,
                dirty INTEGER NOT NULL DEFAULT 1,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS projects (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL DEFAULT '进行中',
                start_ts INTEGER,
                deadline_ts INTEGER,
                note TEXT NOT NULL DEFAULT '',
                notion_page_id TEXT,
                dirty INTEGER NOT NULL DEFAULT 1,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS notes (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                kind TEXT NOT NULL CHECK (kind IN ('dir', 'doc')),
                title TEXT NOT NULL DEFAULT '',
                content_md TEXT NOT NULL DEFAULT '',
                created_ts INTEGER NOT NULL,
                updated_ts INTEGER NOT NULL,
                notion_page_id TEXT,
                dirty INTEGER NOT NULL DEFAULT 1,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS ext_records (
                ns TEXT NOT NULL,
                id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                props_json TEXT NOT NULL DEFAULT '{}',
                content_md TEXT NOT NULL DEFAULT '',
                created_ts INTEGER NOT NULL,
                updated_ts INTEGER NOT NULL,
                notion_page_id TEXT,
                dirty INTEGER NOT NULL DEFAULT 1,
                deleted INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (ns, id)
            );
            CREATE TABLE IF NOT EXISTS ext_namespaces (
                ns TEXT PRIMARY KEY,
                notion_page_id TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS ideas (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                tag TEXT NOT NULL DEFAULT '灵感',
                pinned INTEGER NOT NULL DEFAULT 0,
                created_ts INTEGER,
                updated_ts INTEGER,
                notion_page_id TEXT,
                dirty INTEGER DEFAULT 1,
                deleted INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS tasks (
                id TEXT PRIMARY KEY,
                date TEXT NOT NULL,
                title TEXT NOT NULL,
                priority TEXT NOT NULL DEFAULT '中',
                important INTEGER NOT NULL DEFAULT 0,
                urgent INTEGER NOT NULL DEFAULT 0,
                pomodoro_count INTEGER NOT NULL DEFAULT 0,
                estimated_minutes INTEGER,
                notes TEXT NOT NULL DEFAULT '',
                done INTEGER NOT NULL DEFAULT 0,
                created_ts INTEGER,
                updated_ts INTEGER,
                notion_page_id TEXT,
                dirty INTEGER DEFAULT 1,
                deleted INTEGER DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS sync_meta (
                k TEXT PRIMARY KEY,
                v TEXT NOT NULL DEFAULT ''
            );",
        )?;
        self.migrate()?;
        Ok(())
    }

    /// 旧库补齐后加的列
    fn migrate(&self) -> Result<()> {
        Self::add_column_if_missing(self, "events", "remind", "INTEGER NOT NULL DEFAULT 0")?;
        Self::add_column_if_missing(self, "events", "task_id", "TEXT")?;
        Self::add_column_if_missing(self, "tasks", "important", "INTEGER NOT NULL DEFAULT 0")?;
        Self::add_column_if_missing(self, "tasks", "urgent", "INTEGER NOT NULL DEFAULT 0")?;
        Self::add_column_if_missing(self, "tasks", "pomodoro_count", "INTEGER NOT NULL DEFAULT 0")?;
        Self::add_column_if_missing(self, "tasks", "estimated_minutes", "INTEGER")?;
        Self::add_column_if_missing(self, "tasks", "notes", "TEXT NOT NULL DEFAULT ''")?;
        Self::add_column_if_missing(self, "tasks", "task_type", "TEXT NOT NULL DEFAULT ''")?;
        Self::add_column_if_missing(self, "tasks", "project_id", "TEXT")?;
        Self::add_column_if_missing(self, "tasks", "start_ts", "INTEGER")?;
        // 知识库结构升级（页面树 -> database 行）：清空旧 notion_page_id 并置 dirty，
        // 促使下次同步把笔记重新建到新的「📚 知识库」database。用 user_version 防重复执行
        let uv: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap_or(0);
        if uv < 1 {
            let _ = self
                .conn
                .execute("UPDATE notes SET notion_page_id = NULL, dirty = 1", []);
            self.conn.execute_batch("PRAGMA user_version = 1")?;
        }
        Ok(())
    }

    /// 若表缺少指定列则 ALTER TABLE 补上
    fn add_column_if_missing(&self, table: &str, column: &str, ddl: &str) -> Result<()> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = stmt
            .query_map([], |r| r.get::<_, String>(1))?
            .any(|name| name.is_ok_and(|n| n == column));
        if !exists {
            self.conn
                .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ddl}"))?;
        }
        Ok(())
    }

    // ---------- 事件 ----------

    fn row_to_event(row: &Row) -> rusqlite::Result<Event> {
        Ok(Event {
            id: row.get(0)?,
            start_ts: row.get(1)?,
            end_ts: row.get(2)?,
            content: row.get(3)?,
            tag: Tag::from_label(&row.get::<_, String>(4)?).unwrap_or(Tag::Life),
            notion_page_id: row.get(5)?,
            dirty: row.get::<_, i64>(6)? != 0,
            deleted: row.get::<_, i64>(7)? != 0,
            remind: row.get::<_, i64>(8)? != 0,
            task_id: row.get(9)?,
        })
    }

    /// 开始一个新事件（进行中）
    pub fn create_event(&self, start_ts: i64) -> Result<Event> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO events (id, start_ts) VALUES (?1, ?2)",
            params![id, start_ts],
        )?;
        Ok(Event {
            id,
            start_ts,
            end_ts: None,
            content: String::new(),
            tag: Tag::Life,
            remind: false,
            task_id: None,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    /// 手动补录/规划一个完整事件（end_ts 为 None 表示计划中/进行中）
    pub fn add_event_full(
        &self,
        start_ts: i64,
        end_ts: Option<i64>,
        content: &str,
        tag: Tag,
        remind: bool,
    ) -> Result<Event> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO events (id, start_ts, end_ts, content, tag, remind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, start_ts, end_ts, content, tag.label(), remind],
        )?;
        Ok(Event {
            id,
            start_ts,
            end_ts,
            content: content.to_string(),
            tag,
            remind,
            task_id: None,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    /// 结束事件并填写内容与标签
    pub fn finish_event(&self, id: &str, end_ts: i64, content: &str, tag: Tag) -> Result<()> {
        self.conn.execute(
            "UPDATE events SET end_ts = ?2, content = ?3, tag = ?4, dirty = 1 WHERE id = ?1",
            params![id, end_ts, content, tag.label()],
        )?;
        Ok(())
    }

    /// 关联事件与任务（任务执行计时）
    pub fn set_event_task_id(&self, id: &str, task_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE events SET task_id = ?2, dirty = 1 WHERE id = ?1",
            params![id, task_id],
        )?;
        Ok(())
    }

    /// 当前进行中的事件（最多一个）
    pub fn ongoing_event(&self) -> Result<Option<Event>> {
        let sql =
            format!("SELECT {EVENT_COLS} FROM events WHERE end_ts IS NULL AND deleted = 0 LIMIT 1");
        let ev = self
            .conn
            .query_row(&sql, [], Self::row_to_event)
            .optional()?;
        Ok(ev)
    }

    /// 按 id 查询未删除事件
    pub fn get_event(&self, id: &str) -> Result<Option<Event>> {
        let sql = format!("SELECT {EVENT_COLS} FROM events WHERE id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_event)
            .optional()?)
    }

    /// 部分更新事件；end_ts 传 Some(None) 表示显式清空（重新打开事件）。
    /// 返回是否存在该（未删除）行。有任何字段更新时置 dirty。
    pub fn update_event(
        &self,
        id: &str,
        content: Option<&str>,
        tag: Option<Tag>,
        start_ts: Option<i64>,
        end_ts: Option<Option<i64>>,
        remind: Option<bool>,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(c) = content {
            sets.push("content = ?");
            values.push(Box::new(c.to_string()));
        }
        if let Some(t) = tag {
            sets.push("tag = ?");
            values.push(Box::new(t.label().to_string()));
        }
        if let Some(s) = start_ts {
            sets.push("start_ts = ?");
            values.push(Box::new(s));
        }
        if let Some(e) = end_ts {
            sets.push("end_ts = ?");
            values.push(Box::new(e));
        }
        if let Some(r) = remind {
            sets.push("remind = ?");
            values.push(Box::new(r));
        }
        if sets.is_empty() {
            return Ok(self.get_event(id)?.is_some());
        }
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE events SET {} WHERE id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    /// 与 [from, to) 时间范围有交集的未删除事件；now 用于进行中的事件
    pub fn events_between(&self, from: i64, to: i64, now: i64) -> Result<Vec<Event>> {
        let sql = format!(
            "SELECT {EVENT_COLS} FROM events
             WHERE deleted = 0 AND start_ts < ?2 AND COALESCE(end_ts, ?3) > ?1
             ORDER BY start_ts"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![from, to, now], Self::row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_event(&self, id: &str) -> Result<()> {
        self.soft_or_hard_delete("events", id)
    }

    /// 待同步事件（含 deleted = 1 的）
    pub fn dirty_events(&self) -> Result<Vec<Event>> {
        let sql = format!("SELECT {EVENT_COLS} FROM events WHERE dirty = 1 ORDER BY start_ts");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 到点应提醒的事件：remind=1 且 start_ts 落在上一检查点 (since, now] 内
    pub fn due_reminders(&self, since: i64, now: i64) -> Result<Vec<Event>> {
        let sql = format!(
            "SELECT {EVENT_COLS} FROM events
             WHERE deleted = 0 AND remind = 1 AND start_ts > ?1 AND start_ts <= ?2
             ORDER BY start_ts"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![since, now], Self::row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_event_page_id(&self, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE events SET notion_page_id = ?2, dirty = 0 WHERE id = ?1",
            params![id, page_id],
        )?;
        Ok(())
    }

    pub fn clear_event_dirty(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE events SET dirty = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn purge_event(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM events WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- 消费 ----------

    fn row_to_expense(row: &Row) -> rusqlite::Result<Expense> {
        Ok(Expense {
            id: row.get(0)?,
            item: row.get(1)?,
            amount_cents: row.get(2)?,
            ts: row.get(3)?,
            category: Category::from_label(&row.get::<_, String>(4)?).unwrap_or(Category::Other),
            notion_page_id: row.get(5)?,
            dirty: row.get::<_, i64>(6)? != 0,
            deleted: row.get::<_, i64>(7)? != 0,
        })
    }

    pub fn add_expense(
        &self,
        item: &str,
        amount_cents: i64,
        ts: i64,
        category: Category,
    ) -> Result<Expense> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO expenses (id, item, amount_cents, ts, category) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, item, amount_cents, ts, category.label()],
        )?;
        Ok(Expense {
            id,
            item: item.to_string(),
            amount_cents,
            ts,
            category,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    pub fn all_expenses(&self) -> Result<Vec<Expense>> {
        let sql = format!("SELECT {EXPENSE_COLS} FROM expenses WHERE deleted = 0 ORDER BY ts DESC");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_expense)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// [from, to) 范围内的未删除消费
    pub fn expenses_between(&self, from: i64, to: i64) -> Result<Vec<Expense>> {
        let sql = format!(
            "SELECT {EXPENSE_COLS} FROM expenses
             WHERE deleted = 0 AND ts >= ?1 AND ts < ?2 ORDER BY ts"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![from, to], Self::row_to_expense)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 按 id 查询未删除消费
    pub fn get_expense(&self, id: &str) -> Result<Option<Expense>> {
        let sql = format!("SELECT {EXPENSE_COLS} FROM expenses WHERE id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_expense)
            .optional()?)
    }

    /// 部分更新消费。返回是否存在该（未删除）行。有任何字段更新时置 dirty。
    pub fn update_expense(
        &self,
        id: &str,
        item: Option<&str>,
        amount_cents: Option<i64>,
        ts: Option<i64>,
        category: Option<Category>,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(i) = item {
            sets.push("item = ?");
            values.push(Box::new(i.to_string()));
        }
        if let Some(a) = amount_cents {
            sets.push("amount_cents = ?");
            values.push(Box::new(a));
        }
        if let Some(t) = ts {
            sets.push("ts = ?");
            values.push(Box::new(t));
        }
        if let Some(c) = category {
            sets.push("category = ?");
            values.push(Box::new(c.label().to_string()));
        }
        if sets.is_empty() {
            return Ok(self.get_expense(id)?.is_some());
        }
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE expenses SET {} WHERE id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    pub fn delete_expense(&self, id: &str) -> Result<()> {
        self.soft_or_hard_delete("expenses", id)
    }

    pub fn dirty_expenses(&self) -> Result<Vec<Expense>> {
        let sql = format!("SELECT {EXPENSE_COLS} FROM expenses WHERE dirty = 1 ORDER BY ts");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_expense)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_expense_page_id(&self, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE expenses SET notion_page_id = ?2, dirty = 0 WHERE id = ?1",
            params![id, page_id],
        )?;
        Ok(())
    }

    pub fn clear_expense_dirty(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE expenses SET dirty = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn purge_expense(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM expenses WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- 项目 ----------

    fn row_to_project(row: &Row) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            status: ProjectStatus::from_label(&row.get::<_, String>(2)?)
                .unwrap_or(ProjectStatus::Doing),
            start_ts: row.get(3)?,
            deadline_ts: row.get(4)?,
            note: row.get(5)?,
            notion_page_id: row.get(6)?,
            dirty: row.get::<_, i64>(7)? != 0,
            deleted: row.get::<_, i64>(8)? != 0,
        })
    }

    pub fn add_project(
        &self,
        name: &str,
        status: ProjectStatus,
        start_ts: Option<i64>,
        deadline_ts: Option<i64>,
        note: &str,
    ) -> Result<Project> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO projects (id, name, status, start_ts, deadline_ts, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, name, status.label(), start_ts, deadline_ts, note],
        )?;
        Ok(Project {
            id,
            name: name.to_string(),
            status,
            start_ts,
            deadline_ts,
            note: note.to_string(),
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    pub fn all_projects(&self) -> Result<Vec<Project>> {
        let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE deleted = 0 ORDER BY rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 按 id 查询未删除项目
    pub fn get_project(&self, id: &str) -> Result<Option<Project>> {
        let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_project)
            .optional()?)
    }

    /// 部分更新项目；start_ts / deadline_ts 传 Some(None) 表示显式清空。
    /// 返回是否存在该（未删除）行。有任何字段更新时置 dirty。
    pub fn update_project(
        &self,
        id: &str,
        name: Option<&str>,
        status: Option<ProjectStatus>,
        start_ts: Option<Option<i64>>,
        deadline_ts: Option<Option<i64>>,
        note: Option<&str>,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(n) = name {
            sets.push("name = ?");
            values.push(Box::new(n.to_string()));
        }
        if let Some(s) = status {
            sets.push("status = ?");
            values.push(Box::new(s.label().to_string()));
        }
        if let Some(s) = start_ts {
            sets.push("start_ts = ?");
            values.push(Box::new(s));
        }
        if let Some(d) = deadline_ts {
            sets.push("deadline_ts = ?");
            values.push(Box::new(d));
        }
        if let Some(n) = note {
            sets.push("note = ?");
            values.push(Box::new(n.to_string()));
        }
        if sets.is_empty() {
            return Ok(self.get_project(id)?.is_some());
        }
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE projects SET {} WHERE id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    pub fn set_project_status(&self, id: &str, status: ProjectStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET status = ?2, dirty = 1 WHERE id = ?1",
            params![id, status.label()],
        )?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.soft_or_hard_delete("projects", id)
    }

    pub fn dirty_projects(&self) -> Result<Vec<Project>> {
        let sql = format!("SELECT {PROJECT_COLS} FROM projects WHERE dirty = 1 ORDER BY rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_project_page_id(&self, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE projects SET notion_page_id = ?2, dirty = 0 WHERE id = ?1",
            params![id, page_id],
        )?;
        Ok(())
    }

    pub fn clear_project_dirty(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE projects SET dirty = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn purge_project(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM projects WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- 知识库 ----------

    fn row_to_note(row: &Row) -> rusqlite::Result<Note> {
        Ok(Note {
            id: row.get(0)?,
            parent_id: row.get(1)?,
            kind: NoteKind::from_label(&row.get::<_, String>(2)?).unwrap_or(NoteKind::Doc),
            title: row.get(3)?,
            content_md: row.get(4)?,
            created_ts: row.get(5)?,
            updated_ts: row.get(6)?,
            notion_page_id: row.get(7)?,
            dirty: row.get::<_, i64>(8)? != 0,
            deleted: row.get::<_, i64>(9)? != 0,
        })
    }

    pub fn add_note(
        &self,
        parent_id: Option<&str>,
        kind: NoteKind,
        title: &str,
        content_md: &str,
        now: i64,
    ) -> Result<Note> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO notes (id, parent_id, kind, title, content_md, created_ts, updated_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![id, parent_id, kind.label(), title, content_md, now],
        )?;
        Ok(Note {
            id,
            parent_id: parent_id.map(str::to_string),
            kind,
            title: title.to_string(),
            content_md: content_md.to_string(),
            created_ts: now,
            updated_ts: now,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    /// 按 id 查询未删除条目
    pub fn get_note(&self, id: &str) -> Result<Option<Note>> {
        let sql = format!("SELECT {NOTE_COLS} FROM notes WHERE id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_note)
            .optional()?)
    }

    /// 全部未删除条目（按创建时间排序，供树接口使用）
    pub fn all_notes(&self) -> Result<Vec<Note>> {
        let sql =
            format!("SELECT {NOTE_COLS} FROM notes WHERE deleted = 0 ORDER BY created_ts, rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_note)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 全部条目（含 deleted = 1，供同步使用）
    pub fn all_notes_any(&self) -> Result<Vec<Note>> {
        let sql = format!("SELECT {NOTE_COLS} FROM notes ORDER BY created_ts, rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_note)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 部分更新条目；parent_id 传 Some(None) 表示移到根。
    /// 返回是否存在该（未删除）行。有任何字段更新时刷新 updated_ts 并置 dirty。
    pub fn update_note(
        &self,
        id: &str,
        title: Option<&str>,
        content_md: Option<&str>,
        parent_id: Option<Option<&str>>,
        now: i64,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(t) = title {
            sets.push("title = ?");
            values.push(Box::new(t.to_string()));
        }
        if let Some(c) = content_md {
            sets.push("content_md = ?");
            values.push(Box::new(c.to_string()));
        }
        if let Some(p) = parent_id {
            sets.push("parent_id = ?");
            values.push(Box::new(p.map(str::to_string)));
        }
        if sets.is_empty() {
            return Ok(self.get_note(id)?.is_some());
        }
        sets.push("updated_ts = ?");
        values.push(Box::new(now));
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE notes SET {} WHERE id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    /// 把 note_id 移到 new_parent_id 下是否会成环（沿 new_parent 的祖先链查找 note_id）
    pub fn note_would_cycle(&self, note_id: &str, new_parent_id: &str) -> Result<bool> {
        let mut cur = Some(new_parent_id.to_string());
        while let Some(pid) = cur {
            if pid == note_id {
                return Ok(true);
            }
            cur = self
                .conn
                .query_row(
                    "SELECT parent_id FROM notes WHERE id = ?1",
                    params![pid],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten();
        }
        Ok(false)
    }

    /// 递归删除条目及其全部子孙：已同步过的软删（待远端归档），未同步过的物理删
    pub fn delete_note_recursive(&self, id: &str) -> Result<()> {
        let mut stmt = self.conn.prepare("SELECT id, parent_id FROM notes")?;
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut children: HashMap<String, Vec<String>> = HashMap::new();
        for (nid, pid) in rows {
            if let Some(pid) = pid {
                children.entry(pid).or_default().push(nid);
            }
        }
        let mut stack = vec![id.to_string()];
        let mut all = Vec::new();
        while let Some(cur) = stack.pop() {
            if let Some(kids) = children.get(&cur) {
                stack.extend(kids.iter().cloned());
            }
            all.push(cur);
        }
        for nid in all {
            self.soft_or_hard_delete("notes", &nid)?;
        }
        Ok(())
    }

    /// 待同步条目（含 deleted = 1 的）
    pub fn dirty_notes(&self) -> Result<Vec<Note>> {
        let sql =
            format!("SELECT {NOTE_COLS} FROM notes WHERE dirty = 1 ORDER BY created_ts, rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_note)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_note_page_id(&self, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET notion_page_id = ?2, dirty = 0 WHERE id = ?1",
            params![id, page_id],
        )?;
        Ok(())
    }

    pub fn clear_note_dirty(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE notes SET dirty = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn purge_note(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- 好想法 ----------

    fn row_to_idea(row: &Row) -> rusqlite::Result<Idea> {
        Ok(Idea {
            id: row.get(0)?,
            content: row.get(1)?,
            tag: IdeaTag::from_label(&row.get::<_, String>(2)?).unwrap_or(IdeaTag::Inspiration),
            pinned: row.get::<_, i64>(3)? != 0,
            created_ts: row.get(4)?,
            updated_ts: row.get(5)?,
            notion_page_id: row.get(6)?,
            dirty: row.get::<_, i64>(7)? != 0,
            deleted: row.get::<_, i64>(8)? != 0,
        })
    }

    pub fn add_idea(&self, content: &str, tag: IdeaTag, pinned: bool, now: i64) -> Result<Idea> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO ideas (id, content, tag, pinned, created_ts, updated_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, content, tag.label(), pinned, now],
        )?;
        Ok(Idea {
            id,
            content: content.to_string(),
            tag,
            pinned,
            created_ts: now,
            updated_ts: now,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    /// 全部未删除想法（置顶优先，再按创建时间倒序）；tag 为 Some 时按标签过滤
    pub fn all_ideas(&self, tag: Option<IdeaTag>) -> Result<Vec<Idea>> {
        let (sql, tag_label) = match tag {
            Some(t) => (
                format!(
                    "SELECT {IDEA_COLS} FROM ideas WHERE deleted = 0 AND tag = ?1
                     ORDER BY pinned DESC, created_ts DESC, rowid DESC"
                ),
                Some(t.label().to_string()),
            ),
            None => (
                format!(
                    "SELECT {IDEA_COLS} FROM ideas WHERE deleted = 0
                     ORDER BY pinned DESC, created_ts DESC, rowid DESC"
                ),
                None,
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = match &tag_label {
            Some(l) => stmt.query_map(params![l], Self::row_to_idea)?,
            None => stmt.query_map([], Self::row_to_idea)?,
        };
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 按 id 查询未删除想法
    pub fn get_idea(&self, id: &str) -> Result<Option<Idea>> {
        let sql = format!("SELECT {IDEA_COLS} FROM ideas WHERE id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_idea)
            .optional()?)
    }

    /// 部分更新想法。返回是否存在该（未删除）行。有任何字段更新时刷新 updated_ts 并置 dirty。
    pub fn update_idea(
        &self,
        id: &str,
        content: Option<&str>,
        tag: Option<IdeaTag>,
        pinned: Option<bool>,
        now: i64,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(c) = content {
            sets.push("content = ?");
            values.push(Box::new(c.to_string()));
        }
        if let Some(t) = tag {
            sets.push("tag = ?");
            values.push(Box::new(t.label().to_string()));
        }
        if let Some(p) = pinned {
            sets.push("pinned = ?");
            values.push(Box::new(p));
        }
        if sets.is_empty() {
            return Ok(self.get_idea(id)?.is_some());
        }
        sets.push("updated_ts = ?");
        values.push(Box::new(now));
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE ideas SET {} WHERE id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    pub fn delete_idea(&self, id: &str) -> Result<()> {
        self.soft_or_hard_delete("ideas", id)
    }

    /// 待同步想法（含 deleted = 1 的）
    pub fn dirty_ideas(&self) -> Result<Vec<Idea>> {
        let sql =
            format!("SELECT {IDEA_COLS} FROM ideas WHERE dirty = 1 ORDER BY created_ts, rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_idea)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_idea_page_id(&self, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ideas SET notion_page_id = ?2, dirty = 0 WHERE id = ?1",
            params![id, page_id],
        )?;
        Ok(())
    }
    pub fn clear_idea_dirty(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE ideas SET dirty = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn purge_idea(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM ideas WHERE id = ?1", params![id])?;
        Ok(())
    }

    fn row_to_task(row: &Row) -> rusqlite::Result<Task> {
        Ok(Task {
            id: row.get(0)?,
            date: row.get(1)?,
            title: row.get(2)?,
            priority: TaskPriority::from_label(&row.get::<_, String>(3)?)
                .unwrap_or(TaskPriority::Mid),
            important: row.get::<_, i64>(4)? != 0,
            urgent: row.get::<_, i64>(5)? != 0,
            pomodoro_count: row.get::<_, i32>(6)?,
            estimated_minutes: row.get(7)?,
            notes: row.get(8)?,
            done: row.get::<_, i64>(9)? != 0,
            created_ts: row.get(10)?,
            updated_ts: row.get(11)?,
            notion_page_id: row.get(12)?,
            dirty: row.get::<_, i64>(13)? != 0,
            deleted: row.get::<_, i64>(14)? != 0,
            task_type: row.get(15)?,
            project_id: row.get(16)?,
            start_ts: row.get(17)?,
        })
    }

#[allow(clippy::too_many_arguments)]
    pub fn add_task(
        &self,
        date: &str,
        title: &str,
        priority: TaskPriority,
        important: bool,
        urgent: bool,
        estimated_minutes: Option<i32>,
        notes: &str,
        task_type: &str,
        project_id: Option<&str>,
        start_ts: Option<i64>,
        now: i64,
    ) -> Result<Task> {
        let id = Uuid::new_v4().to_string();
        self.conn.execute(
            "INSERT INTO tasks (id, date, title, priority, important, urgent, estimated_minutes, notes, task_type, project_id, start_ts, created_ts, updated_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
            params![id, date, title, priority.label(), important, urgent, estimated_minutes, notes, task_type, project_id, start_ts, now],
        )?;
        Ok(Task {
            id,
            date: date.to_string(),
            title: title.to_string(),
            priority,
            important,
            urgent,
            pomodoro_count: 0,
            estimated_minutes,
            notes: notes.to_string(),
            task_type: task_type.to_string(),
            project_id: project_id.map(str::to_string),
            start_ts,
            done: false,
            created_ts: now,
            updated_ts: now,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    /// 指定日期的未删除任务：优先级 高>中>低，未完成在前，再按创建时间升序
    pub fn tasks_on(&self, date: &str) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {TASK_COLS} FROM tasks WHERE deleted = 0 AND date = ?1
             ORDER BY done ASC,
                      CASE priority WHEN '高' THEN 0 WHEN '中' THEN 1 ELSE 2 END,
                      created_ts ASC, rowid ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![date], Self::row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 全部未完成任务（不限日期）：优先级 高>中>低，再按计划开始/创建时间升序
    pub fn unfinished_tasks(&self) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {TASK_COLS} FROM tasks WHERE deleted = 0 AND done = 0
             ORDER BY CASE priority WHEN '高' THEN 0 WHEN '中' THEN 1 ELSE 2 END,
                      COALESCE(start_ts, 253402300799) ASC,
                      created_ts ASC, rowid ASC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 任务累计执行秒数（关联事件的时长之和；进行中的事件按 now 计）
    pub fn task_executed_secs(&self, task_id: &str, now: i64) -> Result<i64> {
        let secs: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(end_ts, ?2) - start_ts), 0)
             FROM events WHERE task_id = ?1 AND deleted = 0",
            params![task_id, now],
            |r| r.get(0),
        )?;
        Ok(secs)
    }

    /// 从项目派生提醒任务（不落库）：截止日期落在 [date, date+2] 且未完成的项目，
    /// 生成一条「项目名 截止」提醒。返回按截止时间排序的虚拟任务（id 以 `proj:` 前缀）。
    pub fn project_derived_tasks(&self, date: &str) -> Result<Vec<Task>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, deadline_ts FROM projects
             WHERE deleted = 0 AND status != '已完成' AND deadline_ts IS NOT NULL",
        )?;
        let rows: Vec<(String, String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let target = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .unwrap_or_else(|_| Local::now().date_naive());
        let mut out = Vec::new();
        for (pid, name, deadline_ts) in rows {
            // unix 秒 → 本地日期（时区一致的截止日）
            let d = Local
                .timestamp_opt(deadline_ts, 0)
                .earliest()
                .map(|dt| dt.date_naive())
                .unwrap_or_else(|| Local::now().date_naive());
            // 截止在 [date, date+2] 内才提醒（提前提醒窗口 2 天）
            let window_start = target;
            let window_end = target + chrono::Duration::days(2);
            if d >= window_start && d <= window_end {
                out.push(Task {
                    id: format!("proj:{pid}"),
                    date: date.to_string(),
                    title: format!("📌 {name} 截止"),
                    priority: TaskPriority::High,
                    important: true,
                    urgent: d == target,
                    pomodoro_count: 0,
                    estimated_minutes: None,
                    notes: String::new(),
                    task_type: String::new(),
                    project_id: Some(pid.clone()),
                    start_ts: None,
                    done: false,
                    created_ts: 0,
                    updated_ts: 0,
                    notion_page_id: None,
                    dirty: false,
                    deleted: false,
                });
            }
        }
        out.sort_by_key(|t| t.urgent);
        Ok(out)
    }

    /// 按 id 查询未删除任务
    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let sql = format!("SELECT {TASK_COLS} FROM tasks WHERE id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_task)
            .optional()?)
    }

    /// 部分更新任务。返回是否存在该（未删除）行。有任何字段更新时刷新 updated_ts 并置 dirty。
    #[allow(clippy::too_many_arguments)]
    pub fn update_task(
        &self,
        id: &str,
        title: Option<&str>,
        priority: Option<TaskPriority>,
        important: Option<bool>,
        urgent: Option<bool>,
        pomodoro_count: Option<i32>,
        estimated_minutes: Option<Option<i32>>,
        notes: Option<&str>,
        done: Option<bool>,
        date: Option<&str>,
        task_type: Option<&str>,
        project_id: Option<Option<&str>>,
        start_ts: Option<Option<i64>>,
        now: i64,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(t) = title {
            sets.push("title = ?");
            values.push(Box::new(t.to_string()));
        }
        if let Some(p) = priority {
            sets.push("priority = ?");
            values.push(Box::new(p.label().to_string()));
        }
        if let Some(imp) = important {
            sets.push("important = ?");
            values.push(Box::new(imp));
        }
        if let Some(urg) = urgent {
            sets.push("urgent = ?");
            values.push(Box::new(urg));
        }
        if let Some(pc) = pomodoro_count {
            sets.push("pomodoro_count = ?");
            values.push(Box::new(pc));
        }
        if let Some(em) = estimated_minutes {
            sets.push("estimated_minutes = ?");
            values.push(Box::new(em));
        }
        if let Some(n) = notes {
            sets.push("notes = ?");
            values.push(Box::new(n.to_string()));
        }
        if let Some(d) = done {
            sets.push("done = ?");
            values.push(Box::new(d));
        }
        if let Some(d) = date {
            sets.push("date = ?");
            values.push(Box::new(d.to_string()));
        }
        if let Some(tt) = task_type {
            sets.push("task_type = ?");
            values.push(Box::new(tt.to_string()));
        }
        if let Some(pid) = project_id {
            sets.push("project_id = ?");
            values.push(Box::new(pid.map(str::to_string)));
        }
        if let Some(st) = start_ts {
            sets.push("start_ts = ?");
            values.push(Box::new(st));
        }
        if sets.is_empty() {
            return Ok(self.get_task(id)?.is_some());
        }
        sets.push("updated_ts = ?");
        values.push(Box::new(now));
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE tasks SET {} WHERE id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    /// 递增番茄钟计数（原子操作）
    pub fn increment_pomodoro(&self, id: &str, now: i64) -> Result<bool> {
        let updated = self.conn.execute(
            "UPDATE tasks SET pomodoro_count = pomodoro_count + 1, updated_ts = ?2, dirty = 1
             WHERE id = ?1 AND deleted = 0",
            params![id, now],
        )?;
        Ok(updated > 0)
    }

    pub fn delete_task(&self, id: &str) -> Result<()> {
        self.soft_or_hard_delete("tasks", id)
    }

    /// 将指定日期的未完成任务移到另一日期。返回被移动的任务数。
    pub fn rollover_tasks(&self, from_date: &str, to_date: &str, now: i64) -> Result<usize> {
        let sql = "UPDATE tasks SET date = ?2, updated_ts = ?3, dirty = 1
                   WHERE deleted = 0 AND done = 0 AND date = ?1";
        let count = self.conn.execute(sql, params![from_date, to_date, now])?;
        Ok(count)
    }

    /// 待同步任务（含 deleted = 1 的）
    pub fn dirty_tasks(&self) -> Result<Vec<Task>> {
        let sql = format!("SELECT {TASK_COLS} FROM tasks WHERE dirty = 1 ORDER BY created_ts, rowid");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_task_page_id(&self, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET notion_page_id = ?2, dirty = 0 WHERE id = ?1",
            params![id, page_id],
        )?;
        Ok(())
    }

    pub fn clear_task_dirty(&self, id: &str) -> Result<()> {
        self.conn
            .execute("UPDATE tasks SET dirty = 0 WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn purge_task(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---------- 外部记录（/api/ext） ----------

    fn row_to_ext_record(row: &Row) -> rusqlite::Result<ExtRecord> {
        Ok(ExtRecord {
            ns: row.get(0)?,
            id: row.get(1)?,
            title: row.get(2)?,
            props_json: row.get(3)?,
            content_md: row.get(4)?,
            created_ts: row.get(5)?,
            updated_ts: row.get(6)?,
            notion_page_id: row.get(7)?,
            dirty: row.get::<_, i64>(8)? != 0,
            deleted: row.get::<_, i64>(9)? != 0,
        })
    }

    pub fn insert_ext_record(
        &self,
        ns: &str,
        id: &str,
        title: &str,
        props_json: &str,
        content_md: &str,
        now: i64,
    ) -> Result<ExtRecord> {
        self.conn.execute(
            "INSERT INTO ext_records (ns, id, title, props_json, content_md, created_ts, updated_ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![ns, id, title, props_json, content_md, now],
        )?;
        Ok(ExtRecord {
            ns: ns.to_string(),
            id: id.to_string(),
            title: title.to_string(),
            props_json: props_json.to_string(),
            content_md: content_md.to_string(),
            created_ts: now,
            updated_ts: now,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        })
    }

    /// 按 (ns, id) 查询未删除记录
    pub fn get_ext_record(&self, ns: &str, id: &str) -> Result<Option<ExtRecord>> {
        let sql = format!(
            "SELECT {EXT_RECORD_COLS} FROM ext_records WHERE ns = ?1 AND id = ?2 AND deleted = 0"
        );
        Ok(self
            .conn
            .query_row(&sql, params![ns, id], Self::row_to_ext_record)
            .optional()?)
    }

    /// 命名空间下全部未删除记录（按更新时间倒序）
    pub fn ext_records(&self, ns: &str) -> Result<Vec<ExtRecord>> {
        let sql = format!(
            "SELECT {EXT_RECORD_COLS} FROM ext_records
             WHERE ns = ?1 AND deleted = 0 ORDER BY updated_ts DESC, rowid DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![ns], Self::row_to_ext_record)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 部分更新记录。返回是否存在该（未删除）行。有任何字段更新时刷新 updated_ts 并置 dirty。
    pub fn update_ext_record(
        &self,
        ns: &str,
        id: &str,
        title: Option<&str>,
        props_json: Option<&str>,
        content_md: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let mut sets: Vec<&str> = Vec::new();
        let mut values: Vec<Box<dyn ToSql>> = Vec::new();
        if let Some(t) = title {
            sets.push("title = ?");
            values.push(Box::new(t.to_string()));
        }
        if let Some(p) = props_json {
            sets.push("props_json = ?");
            values.push(Box::new(p.to_string()));
        }
        if let Some(c) = content_md {
            sets.push("content_md = ?");
            values.push(Box::new(c.to_string()));
        }
        if sets.is_empty() {
            return Ok(self.get_ext_record(ns, id)?.is_some());
        }
        sets.push("updated_ts = ?");
        values.push(Box::new(now));
        sets.push("dirty = 1");
        let sql = format!(
            "UPDATE ext_records SET {} WHERE ns = ? AND id = ? AND deleted = 0",
            sets.join(", ")
        );
        values.push(Box::new(ns.to_string()));
        values.push(Box::new(id.to_string()));
        let refs: Vec<&dyn ToSql> = values.iter().map(|v| v.as_ref()).collect();
        Ok(self.conn.execute(&sql, refs.as_slice())? > 0)
    }

    /// 已同步过的记录软删（待远端归档），未同步过的直接物理删
    pub fn delete_ext_record(&self, ns: &str, id: &str) -> Result<()> {
        let page_id: Option<Option<String>> = self
            .conn
            .query_row(
                "SELECT notion_page_id FROM ext_records WHERE ns = ?1 AND id = ?2",
                params![ns, id],
                |r| r.get(0),
            )
            .optional()?;
        match page_id.flatten() {
            Some(_) => {
                self.conn.execute(
                    "UPDATE ext_records SET deleted = 1, dirty = 1 WHERE ns = ?1 AND id = ?2",
                    params![ns, id],
                )?;
            }
            None => {
                self.conn.execute(
                    "DELETE FROM ext_records WHERE ns = ?1 AND id = ?2",
                    params![ns, id],
                )?;
            }
        }
        Ok(())
    }

    /// 待同步记录（含 deleted = 1 的）
    pub fn dirty_ext_records(&self) -> Result<Vec<ExtRecord>> {
        let sql = format!(
            "SELECT {EXT_RECORD_COLS} FROM ext_records WHERE dirty = 1 ORDER BY created_ts, rowid"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_ext_record)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_ext_record_page_id(&self, ns: &str, id: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ext_records SET notion_page_id = ?3, dirty = 0 WHERE ns = ?1 AND id = ?2",
            params![ns, id, page_id],
        )?;
        Ok(())
    }

    pub fn clear_ext_record_dirty(&self, ns: &str, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE ext_records SET dirty = 0 WHERE ns = ?1 AND id = ?2",
            params![ns, id],
        )?;
        Ok(())
    }

    pub fn purge_ext_record(&self, ns: &str, id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ext_records WHERE ns = ?1 AND id = ?2",
            params![ns, id],
        )?;
        Ok(())
    }

    /// 命名空间 → Notion 根页 id 映射（同步用）
    pub fn ext_namespaces(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT ns, notion_page_id FROM ext_namespaces")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_ext_namespace_page_id(&self, ns: &str, page_id: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ext_namespaces (ns, notion_page_id) VALUES (?1, ?2)",
            params![ns, page_id],
        )?;
        Ok(())
    }

    // ---------- 删除公共逻辑 ----------

    /// 待同步（dirty = 1）行总数
    pub fn pending_count(&self) -> Result<usize> {
        let mut total = 0i64;
        for table in [
            "events",
            "expenses",
            "projects",
            "notes",
            "ext_records",
            "ideas",
            "tasks",
        ] {
            let sql = format!("SELECT COUNT(*) FROM {table} WHERE dirty = 1");
            total += self.conn.query_row(&sql, [], |r| r.get::<_, i64>(0))?;
        }
        Ok(total as usize)
    }

    /// 已同步过的行软删除（等同步任务归档远端后清除），未同步过的行直接物理删除
    fn soft_or_hard_delete(&self, table: &str, id: &str) -> Result<()> {
        let sql = format!("SELECT notion_page_id FROM {table} WHERE id = ?1");
        let page_id: Option<Option<String>> = self
            .conn
            .query_row(&sql, params![id], |r| r.get(0))
            .optional()?;
        match page_id.flatten() {
            Some(_) => {
                let sql = format!("UPDATE {table} SET deleted = 1, dirty = 1 WHERE id = ?1");
                self.conn.execute(&sql, params![id])?;
            }
            None => {
                let sql = format!("DELETE FROM {table} WHERE id = ?1");
                self.conn.execute(&sql, params![id])?;
            }
        }
        Ok(())
    }

    /// 清空全部本地业务数据（单事务，失败回滚）。
    ///
    /// 删除 events/expenses/projects/notes/ideas/tasks/ext_records 的全部行；
    /// 保留 ext_namespaces（对外 API 凭证）与 config 中的 Notion token/页面配置。
    /// 用于「清空本地后重新从远端拉取」。
    pub fn clear_all_data(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        let res = (|| {
            for table in [
                "events",
                "expenses",
                "projects",
                "notes",
                "ideas",
                "tasks",
                "ext_records",
            ] {
                let sql = format!("DELETE FROM {table}");
                self.conn.execute(&sql, [])?;
            }
            // 增量拉取水位一并清空：下次增量相当于全量 upsert，可自愈
            self.conn.execute("DELETE FROM sync_meta", [])?;
            Ok(())
        })();
        match res {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// 全部未删除任务（数据导出走这个）
    pub fn all_tasks(&self) -> Result<Vec<Task>> {
        let sql = format!(
            "SELECT {TASK_COLS} FROM tasks WHERE deleted = 0 ORDER BY date DESC, created_ts DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// 全部未删除事件（数据导出走这个）
    pub fn all_events(&self) -> Result<Vec<Event>> {
        let sql = format!(
            "SELECT {EVENT_COLS} FROM events WHERE deleted = 0 ORDER BY start_ts DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ---------- 同步元数据（增量拉取水位等） ----------

    pub fn get_meta(&self, k: &str) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT v FROM sync_meta WHERE k = ?1", params![k], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or_default())
    }

    pub fn set_meta(&self, k: &str, v: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = ?2",
            params![k, v],
        )?;
        Ok(())
    }

    // ---------- 增量拉取 upsert ----------

    /// 按 notion_page_id 找本地行的 (id, dirty)
    fn row_id_dirty_by_page_id(&self, table: &str, page_id: &str) -> Result<Option<(String, bool)>> {
        let sql = format!("SELECT id, dirty FROM {table} WHERE notion_page_id = ?1 AND deleted = 0");
        Ok(self
            .conn
            .query_row(&sql, params![page_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
            })
            .optional()?)
    }

    /// 增量拉取的事件 upsert：新行插入；已同步行更新（remind/task_id 本地专属字段保留）；
    /// 本地 dirty 行跳过（推送优先，推完后远端即为最新，下轮增量自然拉平）
    pub fn upsert_pulled_event(&self, ev: &Event) -> Result<UpsertKind> {
        let Some(pid) = ev.notion_page_id.as_deref() else {
            return Ok(UpsertKind::NoPageId);
        };
        match self.row_id_dirty_by_page_id("events", pid)? {
            Some((_, true)) => Ok(UpsertKind::SkippedDirty),
            Some((id, false)) => {
                self.conn.execute(
                    "UPDATE events SET start_ts=?2, end_ts=?3, content=?4, tag=?5 WHERE id=?1",
                    params![id, ev.start_ts, ev.end_ts, ev.content, ev.tag.label()],
                )?;
                Ok(UpsertKind::Updated)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO events (id, start_ts, end_ts, content, tag, remind, task_id, notion_page_id, dirty, deleted)
                     VALUES (?1,?2,?3,?4,?5,0,NULL,?6,0,0)",
                    params![ev.id, ev.start_ts, ev.end_ts, ev.content, ev.tag.label(), pid],
                )?;
                Ok(UpsertKind::Inserted)
            }
        }
    }

    pub fn upsert_pulled_expense(&self, ex: &Expense) -> Result<UpsertKind> {
        let Some(pid) = ex.notion_page_id.as_deref() else {
            return Ok(UpsertKind::NoPageId);
        };
        match self.row_id_dirty_by_page_id("expenses", pid)? {
            Some((_, true)) => Ok(UpsertKind::SkippedDirty),
            Some((id, false)) => {
                self.conn.execute(
                    "UPDATE expenses SET item=?2, amount_cents=?3, ts=?4, category=?5 WHERE id=?1",
                    params![id, ex.item, ex.amount_cents, ex.ts, ex.category.label()],
                )?;
                Ok(UpsertKind::Updated)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO expenses (id, item, amount_cents, ts, category, notion_page_id, dirty, deleted)
                     VALUES (?1,?2,?3,?4,?5,?6,0,0)",
                    params![ex.id, ex.item, ex.amount_cents, ex.ts, ex.category.label(), pid],
                )?;
                Ok(UpsertKind::Inserted)
            }
        }
    }

    pub fn upsert_pulled_project(&self, p: &Project) -> Result<UpsertKind> {
        let Some(pid) = p.notion_page_id.as_deref() else {
            return Ok(UpsertKind::NoPageId);
        };
        match self.row_id_dirty_by_page_id("projects", pid)? {
            Some((_, true)) => Ok(UpsertKind::SkippedDirty),
            Some((id, false)) => {
                self.conn.execute(
                    "UPDATE projects SET name=?2, status=?3, start_ts=?4, deadline_ts=?5, note=?6 WHERE id=?1",
                    params![id, p.name, p.status.label(), p.start_ts, p.deadline_ts, p.note],
                )?;
                Ok(UpsertKind::Updated)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO projects (id, name, status, start_ts, deadline_ts, note, notion_page_id, dirty, deleted)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,0,0)",
                    params![p.id, p.name, p.status.label(), p.start_ts, p.deadline_ts, p.note, pid],
                )?;
                Ok(UpsertKind::Inserted)
            }
        }
    }

    pub fn upsert_pulled_idea(&self, i: &Idea) -> Result<UpsertKind> {
        let Some(pid) = i.notion_page_id.as_deref() else {
            return Ok(UpsertKind::NoPageId);
        };
        match self.row_id_dirty_by_page_id("ideas", pid)? {
            Some((_, true)) => Ok(UpsertKind::SkippedDirty),
            Some((id, false)) => {
                self.conn.execute(
                    "UPDATE ideas SET content=?2, tag=?3, pinned=?4, created_ts=?5, updated_ts=?6 WHERE id=?1",
                    params![id, i.content, i.tag.label(), i.pinned, i.created_ts, i.updated_ts],
                )?;
                Ok(UpsertKind::Updated)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO ideas (id, content, tag, pinned, created_ts, updated_ts, notion_page_id, dirty, deleted)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,0,0)",
                    params![i.id, i.content, i.tag.label(), i.pinned, i.created_ts, i.updated_ts, pid],
                )?;
                Ok(UpsertKind::Inserted)
            }
        }
    }

    /// 任务 upsert：pomodoro_count/estimated_minutes/notes 为本地专属字段，更新时保留
    pub fn upsert_pulled_task(&self, t: &Task) -> Result<UpsertKind> {
        let Some(pid) = t.notion_page_id.as_deref() else {
            return Ok(UpsertKind::NoPageId);
        };
        match self.row_id_dirty_by_page_id("tasks", pid)? {
            Some((_, true)) => Ok(UpsertKind::SkippedDirty),
            Some((id, false)) => {
                // project_id 用 COALESCE：远端未关联项目（或按名找不到）时保留本地现状
                self.conn.execute(
                    "UPDATE tasks SET date=?2, title=?3, priority=?4, important=?5, urgent=?6, done=?7,
                     created_ts=?8, updated_ts=?9, task_type=?10, project_id=COALESCE(?11, project_id), start_ts=?12 WHERE id=?1",
                    params![
                        id, t.date, t.title, t.priority.label(), t.important, t.urgent, t.done,
                        t.created_ts, t.updated_ts, t.task_type, t.project_id, t.start_ts
                    ],
                )?;
                Ok(UpsertKind::Updated)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO tasks (id, date, title, priority, important, urgent, pomodoro_count, estimated_minutes, notes, done, created_ts, updated_ts, notion_page_id, dirty, deleted, task_type, project_id, start_ts)
                     VALUES (?1,?2,?3,?4,?5,?6,0,NULL,'',?7,?8,?9,?10,0,0,?11,?12,?13)",
                    params![
                        t.id, t.date, t.title, t.priority.label(), t.important, t.urgent, t.done,
                        t.created_ts, t.updated_ts, t.notion_page_id, t.task_type, t.project_id, t.start_ts
                    ],
                )?;
                Ok(UpsertKind::Inserted)
            }
        }
    }

    /// 笔记 upsert：parent_id 由调用方在两阶段处理（本批行的父子关系后补）
    pub fn upsert_pulled_note(&self, n: &Note) -> Result<UpsertKind> {
        let Some(pid) = n.notion_page_id.as_deref() else {
            return Ok(UpsertKind::NoPageId);
        };
        match self.row_id_dirty_by_page_id("notes", pid)? {
            Some((_, true)) => Ok(UpsertKind::SkippedDirty),
            Some((id, false)) => {
                self.conn.execute(
                    "UPDATE notes SET kind=?2, title=?3, content_md=?4, created_ts=?5, updated_ts=?6 WHERE id=?1",
                    params![id, n.kind.label(), n.title, n.content_md, n.created_ts, n.updated_ts],
                )?;
                Ok(UpsertKind::Updated)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO notes (id, parent_id, kind, title, content_md, created_ts, updated_ts, notion_page_id, dirty, deleted)
                     VALUES (?1,NULL,?2,?3,?4,?5,?6,?7,0,0)",
                    params![n.id, n.kind.label(), n.title, n.content_md, n.created_ts, n.updated_ts, pid],
                )?;
                Ok(UpsertKind::Inserted)
            }
        }
    }

    /// 增量拉取第二阶段：按解析出的本地 parent_id 回填（仅未 dirty 的行）
    pub fn set_pulled_note_parent(&self, page_id: &str, parent_id: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE notes SET parent_id = ?2 WHERE notion_page_id = ?1 AND dirty = 0 AND deleted = 0",
            params![page_id, parent_id],
        )?;
        Ok(())
    }

    /// 按 notion_page_id 找笔记的本地 id（父级 relation 解析用）
    pub fn note_id_by_page_id(&self, page_id: &str) -> Result<Option<String>> {
        Ok(self
            .row_id_dirty_by_page_id("notes", page_id)?
            .map(|(id, _)| id))
    }

    /// 按名称找项目 id（增量拉取任务时按「项目」名回填 project_id）
    pub fn project_id_by_name(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM projects WHERE name = ?1 AND deleted = 0 LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .optional()?)
    }

    // ---------- 全局搜索 ----------

    /// 全局搜索：在 6 类实体的标题/正文里做不区分大小写的子串匹配，按类分组返回。
    ///
    /// 没用 FTS5：trigram 分词要求查询词 ≥3 字符，两个汉字（如「内存」）就搜不到；
    /// 本地数据量小，LIKE 子串匹配反而更准、实现更简单。
    pub fn search(&self, kw: &str) -> Result<Vec<SearchHit>> {
        let lower = kw.to_lowercase();
        // 转义 LIKE 特殊字符（配合 ESCAPE '\'）
        let pat = format!(
            "%{}%",
            lower
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        // 每类实体最多返回的条数，避免某类刷爆结果
        const PER_KIND: i64 = 10;
        let mut hits = Vec::new();
        let mut collect = |kind: &'static str, sql: &str, make: &dyn Fn(&rusqlite::Row) -> rusqlite::Result<SearchHit>| -> Result<()> {
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(params![pat, PER_KIND], make)?;
            for h in rows.flatten() {
                hits.push(SearchHit { kind, ..h });
            }
            Ok(())
        };
        collect(
            "note",
            "SELECT id, title, content_md, updated_ts FROM notes
             WHERE deleted = 0 AND (LOWER(title) LIKE ?1 ESCAPE '\\' OR LOWER(content_md) LIKE ?1 ESCAPE '\\')
             ORDER BY updated_ts DESC LIMIT ?2",
            &|r| {
                let title: String = r.get(1)?;
                let body: String = r.get(2)?;
                Ok(SearchHit {
                    kind: "",
                    id: r.get(0)?,
                    snippet: make_snippet(&body, &lower),
                    title,
                    sort_ts: r.get(3)?,
                })
            },
        )?;
        collect(
            "task",
            "SELECT id, title, notes, COALESCE(created_ts, 0) FROM tasks
             WHERE deleted = 0 AND (LOWER(title) LIKE ?1 ESCAPE '\\' OR LOWER(notes) LIKE ?1 ESCAPE '\\')
             ORDER BY COALESCE(created_ts, 0) DESC LIMIT ?2",
            &|r| {
                let title: String = r.get(1)?;
                let body: String = r.get(2)?;
                Ok(SearchHit {
                    kind: "",
                    id: r.get(0)?,
                    snippet: make_snippet(&body, &lower),
                    title,
                    sort_ts: r.get(3)?,
                })
            },
        )?;
        collect(
            "idea",
            "SELECT id, content, COALESCE(created_ts, 0) FROM ideas
             WHERE deleted = 0 AND LOWER(content) LIKE ?1 ESCAPE '\\'
             ORDER BY COALESCE(created_ts, 0) DESC LIMIT ?2",
            &|r| {
                let body: String = r.get(1)?;
                Ok(SearchHit {
                    kind: "",
                    id: r.get(0)?,
                    title: truncate_chars(&body, 20),
                    snippet: make_snippet(&body, &lower),
                    sort_ts: r.get(2)?,
                })
            },
        )?;
        collect(
            "event",
            "SELECT id, content, tag, start_ts FROM events
             WHERE deleted = 0 AND (LOWER(content) LIKE ?1 ESCAPE '\\' OR LOWER(tag) LIKE ?1 ESCAPE '\\')
             ORDER BY start_ts DESC LIMIT ?2",
            &|r| {
                let content: String = r.get(1)?;
                Ok(SearchHit {
                    kind: "",
                    id: r.get(0)?,
                    title: content.clone(),
                    snippet: make_snippet(&content, &lower),
                    sort_ts: r.get(3)?,
                })
            },
        )?;
        collect(
            "expense",
            "SELECT id, item, category, ts FROM expenses
             WHERE deleted = 0 AND (LOWER(item) LIKE ?1 ESCAPE '\\' OR LOWER(category) LIKE ?1 ESCAPE '\\')
             ORDER BY ts DESC LIMIT ?2",
            &|r| {
                let item: String = r.get(1)?;
                Ok(SearchHit {
                    kind: "",
                    id: r.get(0)?,
                    title: item.clone(),
                    snippet: make_snippet(&item, &lower),
                    sort_ts: r.get(3)?,
                })
            },
        )?;
        collect(
            "project",
            "SELECT id, name, note, COALESCE(start_ts, 0) FROM projects
             WHERE deleted = 0 AND (LOWER(name) LIKE ?1 ESCAPE '\\' OR LOWER(note) LIKE ?1 ESCAPE '\\')
             ORDER BY COALESCE(start_ts, 0) DESC LIMIT ?2",
            &|r| {
                let name: String = r.get(1)?;
                let note: String = r.get(2)?;
                Ok(SearchHit {
                    kind: "",
                    id: r.get(0)?,
                    title: name,
                    snippet: make_snippet(&note, &lower),
                    sort_ts: r.get(3)?,
                })
            },
        )?;
        Ok(hits)
    }

    // ---------- 远端拉取覆盖本地 ----------


    /// 用远端拉取的全量数据覆盖本地 5 个表（单事务，失败回滚）。
    ///
    /// 对已同步过的行（按 notion_page_id 匹配）保留本地专属字段：
    /// 事件的 remind/task_id、任务的 pomodoro_count/estimated_minutes/notes。
    /// 其余字段以远端为准；本地存在但远端没有的行被删除（远端为事实来源）。
    pub fn pull_replace_all(&self, data: &PulledData) -> Result<PullCounts> {
        self.conn.execute_batch("BEGIN")?;
        let res = (|| {
            let events = self.replace_events(&data.events)?;
            let expenses = self.replace_expenses(&data.expenses)?;
            let projects = self.replace_projects(&data.projects)?;
            let ideas = self.replace_ideas(&data.ideas)?;
            let tasks = self.replace_tasks(&data.tasks)?;
            let notes = self.replace_notes(&data.notes)?;
            Ok(PullCounts {
                events,
                expenses,
                projects,
                ideas,
                tasks,
                notes,
            })
        })();
        match res {
            Ok(c) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(c)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// 覆盖 events：保留旧行 remind/task_id，其余以远端为准
    fn replace_events(&self, remote: &[Event]) -> Result<usize> {
        let existing: HashMap<String, (bool, Option<String>)> = self
            .conn
            .prepare("SELECT notion_page_id, remind, task_id FROM events WHERE notion_page_id IS NOT NULL")?
            .query_map([], |r| {
                let npid: String = r.get(0)?;
                let remind: i64 = r.get(1).unwrap_or(0);
                let task_id: Option<String> = r.get(2).unwrap_or(None);
                Ok((npid, (remind != 0, task_id)))
            })?
            .filter_map(|r| r.ok())
            .collect();
        self.conn.execute("DELETE FROM events", [])?;
        for ev in remote {
            let (remind, task_id) = ev
                .notion_page_id
                .as_ref()
                .and_then(|id| existing.get(id).cloned())
                .unwrap_or((false, None));
            self.conn.execute(
                "INSERT INTO events (id, start_ts, end_ts, content, tag, remind, task_id, notion_page_id, dirty, deleted)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,0)",
                params![ev.id, ev.start_ts, ev.end_ts, ev.content, ev.tag.label(), remind, task_id, ev.notion_page_id],
            )?;
        }
        Ok(remote.len())
    }

    fn replace_expenses(&self, remote: &[Expense]) -> Result<usize> {
        self.conn.execute("DELETE FROM expenses", [])?;
        for ex in remote {
            self.conn.execute(
                "INSERT INTO expenses (id, item, amount_cents, ts, category, notion_page_id, dirty, deleted)
                 VALUES (?1,?2,?3,?4,?5,?6,0,0)",
                params![ex.id, ex.item, ex.amount_cents, ex.ts, ex.category.label(), ex.notion_page_id],
            )?;
        }
        Ok(remote.len())
    }

    fn replace_projects(&self, remote: &[Project]) -> Result<usize> {
        self.conn.execute("DELETE FROM projects", [])?;
        for p in remote {
            self.conn.execute(
                "INSERT INTO projects (id, name, status, start_ts, deadline_ts, note, notion_page_id, dirty, deleted)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,0,0)",
                params![p.id, p.name, p.status.label(), p.start_ts, p.deadline_ts, p.note, p.notion_page_id],
            )?;
        }
        Ok(remote.len())
    }

    fn replace_ideas(&self, remote: &[Idea]) -> Result<usize> {
        self.conn.execute("DELETE FROM ideas", [])?;
        for i in remote {
            self.conn.execute(
                "INSERT INTO ideas (id, content, tag, pinned, created_ts, updated_ts, notion_page_id, dirty, deleted)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,0,0)",
                params![i.id, i.content, i.tag.label(), i.pinned, i.created_ts, i.updated_ts, i.notion_page_id],
            )?;
        }
        Ok(remote.len())
    }

    /// 覆盖 tasks：保留旧行 pomodoro_count/estimated_minutes/notes，其余以远端为准
    fn replace_tasks(&self, remote: &[Task]) -> Result<usize> {
        let existing: HashMap<String, (i32, Option<i32>, String)> = self
            .conn
            .prepare("SELECT notion_page_id, pomodoro_count, estimated_minutes, notes FROM tasks WHERE notion_page_id IS NOT NULL")?
            .query_map([], |r| {
                let npid: String = r.get(0)?;
                let pomo: i32 = r.get(1).unwrap_or(0);
                let est: Option<i32> = r.get(2).unwrap_or(None);
                let notes: String = r.get(3).unwrap_or_default();
                Ok((npid, (pomo, est, notes)))
            })?
            .filter_map(|r| r.ok())
            .collect();
        self.conn.execute("DELETE FROM tasks", [])?;
        for t in remote {
            let (pomo, est, notes) = t
                .notion_page_id
                .as_ref()
                .and_then(|id| existing.get(id).cloned())
                .unwrap_or((0, None, String::new()));
            self.conn.execute(
                "INSERT INTO tasks (id, date, title, priority, important, urgent, pomodoro_count, estimated_minutes, notes, done, created_ts, updated_ts, notion_page_id, dirty, deleted, task_type, project_id, start_ts)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,0,0,?14,?15,?16)",
                params![t.id, t.date, t.title, t.priority.label(), t.important, t.urgent, pomo, est, notes, t.done, t.created_ts, t.updated_ts, t.notion_page_id, t.task_type, t.project_id, t.start_ts],
            )?;
        }
        Ok(remote.len())
    }

    /// 覆盖 notes：远端为事实来源，全量清空后插入（无本地专属字段需保留）
    fn replace_notes(&self, remote: &[Note]) -> Result<usize> {
        self.conn.execute("DELETE FROM notes", [])?;
        for n in remote {
            self.conn.execute(
                "INSERT INTO notes (id, parent_id, kind, title, content_md, created_ts, updated_ts, notion_page_id, dirty, deleted)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,0)",
                params![n.id, n.parent_id, n.kind.label(), n.title, n.content_md, n.created_ts, n.updated_ts, n.notion_page_id],
            )?;
        }
        Ok(remote.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_lifecycle() {
        let db = Db::in_memory().unwrap();
        let ev = db.create_event(1000).unwrap();
        assert!(ev.dirty);
        assert_eq!(db.ongoing_event().unwrap().unwrap().id, ev.id);

        db.finish_event(&ev.id, 4600, "写代码", Tag::Work).unwrap();
        assert!(db.ongoing_event().unwrap().is_none());

        let list = db.events_between(0, 10000, 10000).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].end_ts, Some(4600));
        assert_eq!(list[0].content, "写代码");
        assert_eq!(list[0].tag, Tag::Work);

        let dirty = db.dirty_events().unwrap();
        assert_eq!(dirty.len(), 1);
        db.set_event_page_id(&ev.id, "page-1").unwrap();
        assert!(db.dirty_events().unwrap().is_empty());
    }

    #[test]
    fn events_between_overlap_semantics() {
        let db = Db::in_memory().unwrap();
        // 跨天事件 23:00 -> 次日 01:00（相对范围）
        let ev = db.create_event(100).unwrap();
        db.finish_event(&ev.id, 300, "x", Tag::Life).unwrap();
        // 范围完全在事件之前
        assert!(db.events_between(0, 100, 1000).unwrap().is_empty());
        // 范围完全在事件之后（start < to 不满足）
        assert!(db.events_between(300, 400, 1000).unwrap().is_empty());
        // 部分重叠
        assert_eq!(db.events_between(0, 150, 1000).unwrap().len(), 1);
        assert_eq!(db.events_between(250, 400, 1000).unwrap().len(), 1);
    }

    #[test]
    fn due_reminders_window_semantics() {
        let db = Db::in_memory().unwrap();
        // remind=1 且 start_ts 落在 (since, now] 才命中
        db.add_event_full(100, Some(200), "不带提醒", Tag::Work, false)
            .unwrap();
        db.add_event_full(300, Some(400), "到点提醒", Tag::Work, true)
            .unwrap();
        // 窗口之外：start_ts <= since 不补发
        assert!(db.due_reminders(300, 500).unwrap().is_empty());
        // 窗口之内（含边界 now）
        let due = db.due_reminders(100, 300).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].content, "到点提醒");
        // start_ts > now 不提前发
        assert!(db.due_reminders(100, 299).unwrap().is_empty());
    }

    #[test]
    fn meta_roundtrip_and_clear_resets() {
        let db = Db::in_memory().unwrap();
        assert_eq!(db.get_meta("incr:events").unwrap(), "");
        db.set_meta("incr:events", "2026-08-01T00:00:00.000Z").unwrap();
        db.set_meta("incr:events", "2026-08-02T00:00:00.000Z").unwrap();
        assert_eq!(db.get_meta("incr:events").unwrap(), "2026-08-02T00:00:00.000Z");
        db.clear_all_data().unwrap();
        assert_eq!(db.get_meta("incr:events").unwrap(), "");
    }

    fn pulled_event(content: &str) -> Event {
        Event {
            id: Uuid::new_v4().to_string(),
            start_ts: 100,
            end_ts: Some(200),
            content: content.into(),
            tag: Tag::Work,
            remind: false,
            task_id: None,
            notion_page_id: Some("p1".into()),
            dirty: false,
            deleted: false,
        }
    }

    #[test]
    fn upsert_event_insert_update_skip_dirty() {
        let db = Db::in_memory().unwrap();
        // 新行插入（dirty=0）
        assert_eq!(db.upsert_pulled_event(&pulled_event("远程事件")).unwrap(), UpsertKind::Inserted);
        let (id, dirty) = db.row_id_dirty_by_page_id("events", "p1").unwrap().unwrap();
        assert!(!dirty);

        // 本地设了 remind（本地专属字段）且已推送；远端更新 → 覆盖内容但保留 remind
        db.update_event(&id, None, None, None, None, Some(true)).unwrap();
        db.clear_event_dirty(&id).unwrap();
        assert_eq!(db.upsert_pulled_event(&pulled_event("远端改名")).unwrap(), UpsertKind::Updated);
        let after = db.get_event(&id).unwrap().unwrap();
        assert_eq!(after.content, "远端改名");
        assert!(after.remind);
        assert!(!after.dirty);

        // 本地 dirty → 跳过（推送优先）
        db.update_event(&id, Some("本地未推送"), None, None, None, None).unwrap();
        assert_eq!(db.upsert_pulled_event(&pulled_event("远端又改")).unwrap(), UpsertKind::SkippedDirty);
        assert_eq!(db.get_event(&id).unwrap().unwrap().content, "本地未推送");
    }

    #[test]
    fn upsert_task_preserves_local_only_fields() {
        let db = Db::in_memory().unwrap();
        let task = |title: &str| Task {
            id: Uuid::new_v4().to_string(),
            date: "2026-08-08".into(),
            title: title.into(),
            priority: crate::models::TaskPriority::Mid,
            important: false,
            urgent: false,
            pomodoro_count: 0,
            estimated_minutes: None,
            notes: String::new(),
            task_type: String::new(),
            project_id: None,
            start_ts: None,
            done: false,
            created_ts: 100,
            updated_ts: 100,
            notion_page_id: Some("tp1".into()),
            dirty: false,
            deleted: false,
        };
        assert_eq!(db.upsert_pulled_task(&task("远程任务")).unwrap(), UpsertKind::Inserted);
        let (id, _) = db.row_id_dirty_by_page_id("tasks", "tp1").unwrap().unwrap();
        // 本地累计了番茄钟/备注（本地专属字段）
        db.increment_pomodoro(&id, 1000).unwrap();
        db.clear_task_dirty(&id).unwrap();
        assert_eq!(db.upsert_pulled_task(&task("远端改名")).unwrap(), UpsertKind::Updated);
        let after = db.get_task(&id).unwrap().unwrap();
        assert_eq!(after.title, "远端改名");
        assert_eq!(after.pomodoro_count, 1); // 本地字段保留
        assert!(!after.dirty);
    }

    #[test]
    fn upsert_note_two_phase_parent() {
        let db = Db::in_memory().unwrap();
        let note = |title: &str, pid: &str| Note {
            id: Uuid::new_v4().to_string(),
            parent_id: None,
            kind: crate::models::NoteKind::Dir,
            title: title.into(),
            content_md: String::new(),
            created_ts: 100,
            updated_ts: 100,
            notion_page_id: Some(pid.into()),
            dirty: false,
            deleted: false,
        };
        // 子先插入（parent 未解析），再插父，第二阶段回填
        assert_eq!(db.upsert_pulled_note(&note("子", "np-child")).unwrap(), UpsertKind::Inserted);
        assert_eq!(db.upsert_pulled_note(&note("父", "np-parent")).unwrap(), UpsertKind::Inserted);
        let parent_local = db.note_id_by_page_id("np-parent").unwrap().unwrap();
        db.set_pulled_note_parent("np-child", Some(&parent_local)).unwrap();
        let child_local = db.note_id_by_page_id("np-child").unwrap().unwrap();
        let child = db.get_note(&child_local).unwrap().unwrap();
        assert_eq!(child.parent_id.as_deref(), Some(parent_local.as_str()));
    }

    #[test]
    fn delete_unsynced_row_is_hard() {
        let db = Db::in_memory().unwrap();
        let ev = db.create_event(1000).unwrap();
        db.delete_event(&ev.id).unwrap();
        assert!(db.events_between(0, 99999, 99999).unwrap().is_empty());
        // 物理删除，不产生 dirty 行
        assert!(db.dirty_events().unwrap().is_empty());
    }

    #[test]
    fn delete_synced_row_is_soft_then_purge() {
        let db = Db::in_memory().unwrap();
        let ev = db.create_event(1000).unwrap();
        db.finish_event(&ev.id, 2000, "x", Tag::Life).unwrap();
        db.set_event_page_id(&ev.id, "page-1").unwrap();
        db.delete_event(&ev.id).unwrap();
        // 列表中不可见，但 dirty 行中包含 deleted 记录
        assert!(db.events_between(0, 99999, 99999).unwrap().is_empty());
        let dirty = db.dirty_events().unwrap();
        assert_eq!(dirty.len(), 1);
        assert!(dirty[0].deleted);
        db.purge_event(&ev.id).unwrap();
        assert!(db.dirty_events().unwrap().is_empty());
    }

    #[test]
    fn expense_crud() {
        let db = Db::in_memory().unwrap();
        let e1 = db.add_expense("午饭", 2500, 100, Category::Food).unwrap();
        let _e2 = db
            .add_expense("地铁", 400, 200, Category::Transport)
            .unwrap();
        let all = db.all_expenses().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].item, "地铁"); // 按时间倒序
        let ranged = db.expenses_between(0, 150).unwrap();
        assert_eq!(ranged.len(), 1);
        assert_eq!(ranged[0].amount_cents, 2500);

        db.set_expense_page_id(&e1.id, "p").unwrap();
        db.delete_expense(&e1.id).unwrap();
        assert_eq!(db.all_expenses().unwrap().len(), 1);
        let dirty = db.dirty_expenses().unwrap();
        assert!(dirty.iter().any(|e| e.deleted));
    }

    #[test]
    fn project_crud_and_status() {
        let db = Db::in_memory().unwrap();
        let p = db
            .add_project("Latte", ProjectStatus::Doing, Some(100), None, "备注")
            .unwrap();
        let list = db.all_projects().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].status, ProjectStatus::Doing);

        db.set_project_status(&p.id, ProjectStatus::Done).unwrap();
        let list = db.all_projects().unwrap();
        assert_eq!(list[0].status, ProjectStatus::Done);
        assert!(db.dirty_projects().unwrap().iter().any(|x| x.id == p.id));

        db.clear_project_dirty(&p.id).unwrap();
        assert!(db.dirty_projects().unwrap().is_empty());

        db.delete_project(&p.id).unwrap();
        assert!(db.all_projects().unwrap().is_empty());
    }

    #[test]
    fn update_event_partial_fields() {
        let db = Db::in_memory().unwrap();
        let ev = db.create_event(1000).unwrap();
        db.finish_event(&ev.id, 2000, "旧内容", Tag::Life).unwrap();
        db.clear_event_dirty(&ev.id).unwrap();

        // 只改内容，其余字段不变，且重新置 dirty
        assert!(
            db.update_event(&ev.id, Some("新内容"), None, None, None, None)
                .unwrap()
        );
        let e = db.get_event(&ev.id).unwrap().unwrap();
        assert_eq!(e.content, "新内容");
        assert_eq!(e.tag, Tag::Life);
        assert_eq!(e.start_ts, 1000);
        assert_eq!(e.end_ts, Some(2000));
        assert!(e.dirty);

        // 显式清空 end_ts（重新打开事件），并打开提醒
        db.update_event(
            &ev.id,
            None,
            Some(Tag::Work),
            Some(500),
            Some(None),
            Some(true),
        )
        .unwrap();
        let e = db.get_event(&ev.id).unwrap().unwrap();
        assert_eq!(e.content, "新内容");
        assert_eq!(e.tag, Tag::Work);
        assert_eq!(e.start_ts, 500);
        assert_eq!(e.end_ts, None);
        assert!(e.remind);

        // 不存在的 id 返回 false
        assert!(
            !db.update_event("nope", Some("x"), None, None, None, None)
                .unwrap()
        );
    }

    #[test]
    fn add_event_full_roundtrip() {
        let db = Db::in_memory().unwrap();
        let ev = db
            .add_event_full(1000, Some(2000), "补录会议", Tag::Work, true)
            .unwrap();
        assert!(ev.dirty);
        let e = db.get_event(&ev.id).unwrap().unwrap();
        assert_eq!(e.start_ts, 1000);
        assert_eq!(e.end_ts, Some(2000));
        assert_eq!(e.content, "补录会议");
        assert_eq!(e.tag, Tag::Work);
        assert!(e.remind);

        // end_ts 为空的计划事件会被 ongoing_event 视为进行中
        let ev2 = db
            .add_event_full(3000, None, "计划", Tag::Reading, false)
            .unwrap();
        assert_eq!(db.ongoing_event().unwrap().unwrap().id, ev2.id);
        assert!(!db.get_event(&ev2.id).unwrap().unwrap().remind);
    }

    #[test]
    fn migrate_adds_remind_column_to_old_db() {
        let path = std::env::temp_dir().join(format!("latte-test-{}.db", Uuid::new_v4()));
        // 先造一个旧版 schema（无 remind 列）的库，并写入一行
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                    id TEXT PRIMARY KEY,
                    start_ts INTEGER NOT NULL,
                    end_ts INTEGER,
                    content TEXT NOT NULL DEFAULT '',
                    tag TEXT NOT NULL DEFAULT '生活',
                    notion_page_id TEXT,
                    dirty INTEGER NOT NULL DEFAULT 1,
                    deleted INTEGER NOT NULL DEFAULT 0
                );
                INSERT INTO events (id, start_ts, end_ts, content, tag)
                VALUES ('old', 1000, 2000, '旧事', '工作');",
            )
            .unwrap();
        }
        // 打开后应自动迁移：旧行 remind 默认为 false，新写入可带 remind
        let db = Db::open(&path).unwrap();
        let old = db.get_event("old").unwrap().unwrap();
        assert!(!old.remind);
        assert_eq!(old.content, "旧事");
        let ev = db
            .add_event_full(3000, None, "新事", Tag::Life, true)
            .unwrap();
        assert!(db.get_event(&ev.id).unwrap().unwrap().remind);
        drop(db);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn update_expense_partial_fields() {
        let db = Db::in_memory().unwrap();
        let ex = db.add_expense("午饭", 2500, 100, Category::Food).unwrap();
        assert!(
            db.update_expense(&ex.id, None, Some(3000), None, Some(Category::Fun))
                .unwrap()
        );
        let e = db.get_expense(&ex.id).unwrap().unwrap();
        assert_eq!(e.item, "午饭");
        assert_eq!(e.amount_cents, 3000);
        assert_eq!(e.ts, 100);
        assert_eq!(e.category, Category::Fun);
        assert!(
            !db.update_expense("nope", Some("x"), None, None, None)
                .unwrap()
        );
    }

    #[test]
    fn update_project_partial_fields() {
        let db = Db::in_memory().unwrap();
        let p = db
            .add_project("Latte", ProjectStatus::Doing, Some(100), None, "备注")
            .unwrap();
        db.update_project(
            &p.id,
            None,
            Some(ProjectStatus::Paused),
            None,
            Some(Some(999)),
            None,
        )
        .unwrap();
        let p2 = db.get_project(&p.id).unwrap().unwrap();
        assert_eq!(p2.name, "Latte");
        assert_eq!(p2.status, ProjectStatus::Paused);
        assert_eq!(p2.start_ts, Some(100));
        assert_eq!(p2.deadline_ts, Some(999));
        assert_eq!(p2.note, "备注");

        // 显式清空 deadline
        db.update_project(&p.id, None, None, None, Some(None), None)
            .unwrap();
        assert_eq!(db.get_project(&p.id).unwrap().unwrap().deadline_ts, None);
        assert!(
            !db.update_project("nope", Some("x"), None, None, None, None)
                .unwrap()
        );
    }

    #[test]
    fn note_crud_and_tree_order() {
        let db = Db::in_memory().unwrap();
        let dir = db.add_note(None, NoteKind::Dir, "根目录", "", 100).unwrap();
        assert!(dir.dirty);
        let doc = db
            .add_note(Some(&dir.id), NoteKind::Doc, "文档", "# 你好", 200)
            .unwrap();
        let all = db.all_notes().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, dir.id); // 按 created_ts 排序
        assert_eq!(all[1].parent_id.as_deref(), Some(dir.id.as_str()));
        assert_eq!(all[1].kind, NoteKind::Doc);

        // 部分更新：改标题 + 移到根
        db.clear_note_dirty(&doc.id).unwrap();
        assert!(
            db.update_note(&doc.id, Some("新标题"), None, Some(None), 300)
                .unwrap()
        );
        let n = db.get_note(&doc.id).unwrap().unwrap();
        assert_eq!(n.title, "新标题");
        assert_eq!(n.content_md, "# 你好");
        assert_eq!(n.parent_id, None);
        assert_eq!(n.updated_ts, 300);
        assert!(n.dirty);
        assert!(!db.update_note("nope", Some("x"), None, None, 400).unwrap());
    }

    #[test]
    fn note_cycle_detection() {
        let db = Db::in_memory().unwrap();
        let a = db.add_note(None, NoteKind::Dir, "A", "", 1).unwrap();
        let b = db.add_note(Some(&a.id), NoteKind::Dir, "B", "", 2).unwrap();
        let c = db.add_note(Some(&b.id), NoteKind::Doc, "C", "", 3).unwrap();

        // A 移到自己或子孙 B 下 → 成环
        assert!(db.note_would_cycle(&a.id, &a.id).unwrap());
        assert!(db.note_would_cycle(&a.id, &b.id).unwrap());
        // C 移到 A 下（跨分支）、B 保持在 A 下 → 不成环
        assert!(!db.note_would_cycle(&c.id, &a.id).unwrap());
        assert!(!db.note_would_cycle(&b.id, &a.id).unwrap());
        // 不存在的 parent 不成环（由 API 层另外校验存在性）
        assert!(!db.note_would_cycle(&a.id, "ghost").unwrap());
    }

    #[test]
    fn note_recursive_delete_soft_and_hard() {
        let db = Db::in_memory().unwrap();
        let dir = db.add_note(None, NoteKind::Dir, "A", "", 1).unwrap();
        let sub = db
            .add_note(Some(&dir.id), NoteKind::Dir, "B", "", 2)
            .unwrap();
        let doc = db
            .add_note(Some(&sub.id), NoteKind::Doc, "C", "", 3)
            .unwrap();
        // dir 已同步过，sub/doc 未同步
        db.set_note_page_id(&dir.id, "page-a").unwrap();

        db.delete_note_recursive(&dir.id).unwrap();
        // 全部从可见列表消失
        assert!(db.all_notes().unwrap().is_empty());
        assert!(db.get_note(&doc.id).unwrap().is_none());
        // dir 软删（dirty + deleted），sub/doc 物理删除
        let dirty = db.dirty_notes().unwrap();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].id, dir.id);
        assert!(dirty[0].deleted);
        assert!(db.all_notes_any().unwrap().iter().all(|n| n.id == dir.id));
        db.purge_note(&dir.id).unwrap();
        assert!(db.all_notes_any().unwrap().is_empty());
    }

    #[test]
    fn old_db_without_notes_table_opens_and_works() {
        let path = std::env::temp_dir().join(format!("latte-test-{}.db", Uuid::new_v4()));
        // 旧版 schema：只有 3 张表，无 notes
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE events (
                    id TEXT PRIMARY KEY,
                    start_ts INTEGER NOT NULL,
                    end_ts INTEGER,
                    content TEXT NOT NULL DEFAULT '',
                    tag TEXT NOT NULL DEFAULT '生活',
                    notion_page_id TEXT,
                    dirty INTEGER NOT NULL DEFAULT 1,
                    deleted INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE expenses (
                    id TEXT PRIMARY KEY,
                    item TEXT NOT NULL DEFAULT '',
                    amount_cents INTEGER NOT NULL DEFAULT 0,
                    ts INTEGER NOT NULL,
                    category TEXT NOT NULL DEFAULT '其他',
                    notion_page_id TEXT,
                    dirty INTEGER NOT NULL DEFAULT 1,
                    deleted INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE projects (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL DEFAULT '',
                    status TEXT NOT NULL DEFAULT '进行中',
                    start_ts INTEGER,
                    deadline_ts INTEGER,
                    note TEXT NOT NULL DEFAULT '',
                    notion_page_id TEXT,
                    dirty INTEGER NOT NULL DEFAULT 1,
                    deleted INTEGER NOT NULL DEFAULT 0
                );",
            )
            .unwrap();
        }
        // 打开后 notes 表自动建好，可正常读写
        let db = Db::open(&path).unwrap();
        let dir = db.add_note(None, NoteKind::Dir, "目录", "", 1).unwrap();
        assert_eq!(db.all_notes().unwrap().len(), 1);
        assert_eq!(db.pending_count().unwrap(), 1);
        db.delete_note_recursive(&dir.id).unwrap();
        assert_eq!(db.pending_count().unwrap(), 0);
        drop(db);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ext_record_upsert_list_delete() {
        let db = Db::in_memory().unwrap();
        let r1 = db
            .insert_ext_record("agents", "task-1", "任务一", "{}", "", 100)
            .unwrap();
        assert!(r1.dirty);
        db.insert_ext_record("agents", "task-2", "任务二", "{}", "正文", 200)
            .unwrap();
        // 其他命名空间互不影响
        db.insert_ext_record("wiki", "task-1", "同名", "{}", "", 300)
            .unwrap();

        // 列表按 updated_ts 倒序
        let list = db.ext_records("agents").unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "task-2");
        assert_eq!(list[1].id, "task-1");

        // upsert 更新：只改出现的字段，刷 updated_ts/dirty
        db.clear_ext_record_dirty("agents", "task-1").unwrap();
        assert!(
            db.update_ext_record(
                "agents",
                "task-1",
                Some("改名"),
                Some("{\"a\":1}"),
                None,
                400
            )
            .unwrap()
        );
        let r = db.get_ext_record("agents", "task-1").unwrap().unwrap();
        assert_eq!(r.title, "改名");
        assert_eq!(r.props_json, "{\"a\":1}");
        assert_eq!(r.content_md, "");
        assert_eq!(r.created_ts, 100);
        assert_eq!(r.updated_ts, 400);
        assert!(r.dirty);
        assert_eq!(db.ext_records("agents").unwrap()[0].id, "task-1");
        assert!(
            !db.update_ext_record("agents", "nope", Some("x"), None, None, 500)
                .unwrap()
        );

        // 删除语义：未同步物理删，已同步软删 + purge
        db.delete_ext_record("agents", "task-2").unwrap();
        assert!(db.get_ext_record("agents", "task-2").unwrap().is_none());
        assert!(
            db.dirty_ext_records()
                .unwrap()
                .iter()
                .all(|r| r.id != "task-2")
        );
        db.set_ext_record_page_id("agents", "task-1", "page-1")
            .unwrap();
        db.delete_ext_record("agents", "task-1").unwrap();
        assert!(db.ext_records("agents").unwrap().is_empty());
        // dirty 行：agents/task-1 软删（wiki/task-1 从未清过 dirty，也在其中）
        let dirty = db.dirty_ext_records().unwrap();
        let deleted: Vec<_> = dirty.iter().filter(|r| r.deleted).collect();
        assert_eq!(deleted.len(), 1);
        assert_eq!(deleted[0].ns, "agents");
        db.purge_ext_record("agents", "task-1").unwrap();
        assert!(
            db.dirty_ext_records()
                .unwrap()
                .iter()
                .all(|r| r.ns == "wiki")
        );
        // wiki 命名空间的同名记录不受影响
        assert_eq!(db.ext_records("wiki").unwrap().len(), 1);
    }

    #[test]
    fn ext_namespace_page_id_roundtrip() {
        let db = Db::in_memory().unwrap();
        assert!(db.ext_namespaces().unwrap().is_empty());
        db.set_ext_namespace_page_id("agents", "page-ns").unwrap();
        assert_eq!(
            db.ext_namespaces().unwrap(),
            vec![("agents".to_string(), "page-ns".to_string())]
        );
    }

    #[test]
    fn idea_crud_order_and_tag_filter() {
        let db = Db::in_memory().unwrap();
        let i1 = db
            .add_idea("旧灵感", IdeaTag::Inspiration, false, 100)
            .unwrap();
        let i2 = db.add_idea("新待办", IdeaTag::Todo, false, 200).unwrap();
        let i3 = db.add_idea("置顶读书", IdeaTag::Reading, true, 50).unwrap();
        assert!(i1.dirty && !i1.pinned);

        // 排序：置顶优先，其余按 created_ts 倒序
        let all = db.all_ideas(None).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, i3.id);
        assert_eq!(all[1].id, i2.id);
        assert_eq!(all[2].id, i1.id);

        // tag 过滤
        let todos = db.all_ideas(Some(IdeaTag::Todo)).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].content, "新待办");

        // 部分更新：只改 pinned，刷 updated_ts 并重新置 dirty
        db.clear_idea_dirty(&i2.id).unwrap();
        assert!(db.update_idea(&i2.id, None, None, Some(true), 300).unwrap());
        let idea = db.get_idea(&i2.id).unwrap().unwrap();
        assert_eq!(idea.content, "新待办");
        assert_eq!(idea.tag, IdeaTag::Todo);
        assert!(idea.pinned);
        assert_eq!(idea.created_ts, 200);
        assert_eq!(idea.updated_ts, 300);
        assert!(idea.dirty);
        // 改内容与标签
        assert!(
            db.update_idea(&i2.id, Some("改内容"), Some(IdeaTag::Question), None, 400)
                .unwrap()
        );
        let idea = db.get_idea(&i2.id).unwrap().unwrap();
        assert_eq!(idea.content, "改内容");
        assert_eq!(idea.tag, IdeaTag::Question);
        assert!(!db.update_idea("nope", Some("x"), None, None, 500).unwrap());
        assert!(db.get_idea("nope").unwrap().is_none());

        // 删除语义：未同步物理删，已同步软删 + purge
        db.delete_idea(&i1.id).unwrap();
        assert!(db.get_idea(&i1.id).unwrap().is_none());
        assert!(db.dirty_ideas().unwrap().iter().all(|i| i.id != i1.id));
        db.set_idea_page_id(&i3.id, "page-1").unwrap();
        db.delete_idea(&i3.id).unwrap();
        assert_eq!(db.all_ideas(None).unwrap().len(), 1);
        let dirty = db.dirty_ideas().unwrap();
        assert!(dirty.iter().any(|i| i.id == i3.id && i.deleted));
        db.purge_idea(&i3.id).unwrap();
        assert!(db.dirty_ideas().unwrap().iter().all(|i| i.id != i3.id));

        // pending_count 计入 ideas
        assert!(db.pending_count().unwrap() > 0);
        let dirty_ids: Vec<String> = db
            .dirty_ideas()
            .unwrap()
            .iter()
            .map(|i| i.id.clone())
            .collect();
        for id in dirty_ids {
            db.clear_idea_dirty(&id).unwrap();
        }
        assert_eq!(db.pending_count().unwrap(), 0);
    }

    #[test]
    fn pending_count_counts_dirty_rows() {
        let db = Db::in_memory().unwrap();
        assert_eq!(db.pending_count().unwrap(), 0);
        let ev = db.create_event(1000).unwrap();
        let ex = db.add_expense("x", 100, 100, Category::Other).unwrap();
        assert_eq!(db.pending_count().unwrap(), 2);
        db.clear_event_dirty(&ev.id).unwrap();
        db.clear_expense_dirty(&ex.id).unwrap();
        assert_eq!(db.pending_count().unwrap(), 0);
    }

    #[test]
    fn task_crud_order_filter_and_delete() {
        let db = Db::in_memory().unwrap();
        let t1 = db
            .add_task("2024-08-01", "低优先", TaskPriority::Low, false, false, None, "", "", None, None, 100)
            .unwrap();
        let t2 = db
            .add_task("2024-08-01", "高优先", TaskPriority::High, false, false, None, "", "", None, None, 200)
            .unwrap();
        let t3 = db
            .add_task("2024-08-01", "中优先", TaskPriority::Mid, false, false, None, "", "", None, None, 300)
            .unwrap();
        let _other_day = db
            .add_task("2024-08-02", "明天的", TaskPriority::High, false, false, None, "", "", None, None, 400)
            .unwrap();
        assert!(t1.dirty && !t1.done);

        // 按日期过滤 + 排序：高 > 中 > 低
        let list = db.tasks_on("2024-08-01").unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, t2.id);
        assert_eq!(list[1].id, t3.id);
        assert_eq!(list[2].id, t1.id);

        // 完成的排到最后（同级优先级内未完成在前）
        db.update_task(&t2.id, None, None, None, None, None, None, None, Some(true), None, None, None, None, 500)
            .unwrap();
        let list = db.tasks_on("2024-08-01").unwrap();
        assert_eq!(list[0].id, t3.id);
        assert_eq!(list[1].id, t1.id);
        assert_eq!(list[2].id, t2.id);

        // 部分更新：改标题/日期/优先级，刷 updated_ts 并重新置 dirty
        db.clear_task_dirty(&t1.id).unwrap();
        assert!(
            db.update_task(
                &t1.id,
                Some("改名"),
                Some(TaskPriority::High),
                None,
                None,
                None,
                None,
                None,
                None,
                Some("2024-08-03"),
                None,
                None,
                None,
                600,
            )
            .unwrap()
        );
        let task = db.get_task(&t1.id).unwrap().unwrap();
        assert_eq!(task.title, "改名");
        assert_eq!(task.priority, TaskPriority::High);
        assert_eq!(task.date, "2024-08-03");
        assert_eq!(task.created_ts, 100);
        assert_eq!(task.updated_ts, 600);
        assert!(
            !db.update_task("nope", Some("x"), None, None, None, None, None, None, None, None, None, None, None, 700)
                .unwrap()
        );
        // 删除语义：未同步物理删，已同步软删 + purge
        db.delete_task(&t3.id).unwrap();
        assert!(db.get_task(&t3.id).unwrap().is_none());
        assert!(db.dirty_tasks().unwrap().iter().all(|t| t.id != t3.id));
        db.set_task_page_id(&t2.id, "page-1").unwrap();
        db.delete_task(&t2.id).unwrap();
        assert_eq!(db.tasks_on("2024-08-01").unwrap().len(), 0);
        let dirty = db.dirty_tasks().unwrap();
        assert!(dirty.iter().any(|t| t.id == t2.id && t.deleted));
        db.purge_task(&t2.id).unwrap();
        assert!(db.dirty_tasks().unwrap().iter().all(|t| t.id != t2.id));

        // pending_count 计入 tasks
        assert!(db.pending_count().unwrap() > 0);
        let dirty_ids: Vec<String> = db
            .dirty_tasks()
            .unwrap()
            .iter()
            .map(|t| t.id.clone())
            .collect();
        for id in dirty_ids {
            db.clear_task_dirty(&id).unwrap();
        }
        assert_eq!(db.pending_count().unwrap(), 0);
    }

    #[test]
    fn task_rollover_moves_only_undone() {
        let db = Db::in_memory().unwrap();
        let undo = db
            .add_task("2024-08-01", "未完成", TaskPriority::High, true, false, None, "", "", None, None, 100)
            .unwrap();
        let done = db
            .add_task("2024-08-01", "已完成", TaskPriority::Mid, false, false, None, "", "", None, None, 200)
            .unwrap();
        db.update_task(&done.id, None, None, None, None, None, None, None, Some(true), None, None, None, None, 300)
            .unwrap();
        let other = db
            .add_task("2024-08-02", "别的日子", TaskPriority::Low, false, false, None, "", "", None, None, 400)
            .unwrap();

        // 只移动未完成的，已完成与其它日期不受影响
        let moved = db.rollover_tasks("2024-08-01", "2024-08-02", 500).unwrap();
        assert_eq!(moved, 1);

        let task = db.get_task(&undo.id).unwrap().unwrap();
        assert_eq!(task.date, "2024-08-02");
        assert_eq!(task.updated_ts, 500);
        assert!(task.dirty);
        let done_task = db.get_task(&done.id).unwrap().unwrap();
        assert_eq!(done_task.date, "2024-08-01");
        let other_task = db.get_task(&other.id).unwrap().unwrap();
        assert_eq!(other_task.date, "2024-08-02");

        // 无未完成任务时返回 0
        assert_eq!(db.rollover_tasks("2024-08-01", "2024-08-02", 600).unwrap(), 0);
    }

    #[test]
    fn task_important_urgent_fields_roundtrip() {
        let db = Db::in_memory().unwrap();
        let t = db
            .add_task("2024-08-01", "重要且紧急", TaskPriority::High, true, true, None, "", "", None, None, 100)
            .unwrap();
        assert!(t.important && t.urgent);
        let got = db.get_task(&t.id).unwrap().unwrap();
        assert!(got.important && got.urgent);

        // 部分更新只改 urgent
        db.update_task(&t.id, None, None, Some(false), None, None, None, None, None, None, None, None, None, 200)
            .unwrap();
        let got = db.get_task(&t.id).unwrap().unwrap();
        assert!(!got.important && got.urgent);
    }
    #[test]
    fn project_derived_tasks_window() {
        let db = Db::in_memory().unwrap();
        // 截止今天（unix 秒，取当天 12:00）
        let today = Local::now().date_naive();
        let today_ts = today
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        // 截止 5 天后（窗口外）
        let far_ts = (today + chrono::Duration::days(5))
            .and_hms_opt(12, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        db.add_project("截止今天", ProjectStatus::Doing, None, Some(today_ts), "")
            .unwrap();
        db.add_project("窗口外", ProjectStatus::Doing, None, Some(far_ts), "")
            .unwrap();
        // 已完成项目不提醒
        db.add_project("已完成", ProjectStatus::Done, None, Some(today_ts), "")
            .unwrap();

        let date = today.format("%Y-%m-%d").to_string();
        let derived = db.project_derived_tasks(&date).unwrap();
        assert_eq!(derived.len(), 1);
        assert_eq!(derived[0].title, "📌 截止今天 截止");
        assert!(derived[0].id.starts_with("proj:"));
        assert!(derived[0].important);
        assert!(derived[0].urgent); // 截止当天 → 紧急

        // 提前 1 天也在窗口内，且不紧急
        let date2 = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();
        let derived = db.project_derived_tasks(&date2).unwrap();
        assert_eq!(derived.len(), 1);
        assert!(!derived[0].urgent);
    }
}
