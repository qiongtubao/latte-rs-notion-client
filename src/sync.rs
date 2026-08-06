//! 后台同步：每 30s 扫描 dirty 行推送到 Notion。
//!
//! - notion_page_id 为空 → POST 新建并回填 id
//! - 已有 notion_page_id → PATCH 更新
//! - deleted = 1 → 归档远端（archived: true）后物理删除本地行
//! - 单行失败保留 dirty，下轮重试，不影响其他行
//! - 知识库（notes）按「父先子后」处理：父还没同步出 page id 的子行本轮跳过；
//!   config 缺 notes_db_id 时先补建「📚 知识库」database（含自关联「父级」）并持久化
//! - 好想法（ideas）：config 缺 ideas_db_id 时先补建「好想法」database 并持久化，
//!   建不出则整批记 failed
//! - 今日任务（tasks）：同 ideas，config 缺 tasks_db_id 时懒建「今日任务」database

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use tokio::sync::Notify;

use crate::config::{self, Config};
use crate::db::Db;
use crate::models::{Event, Expense, ExtRecord, Idea, Note, NoteKind, Project, Task};
use crate::notion::{self, NotionClient};

const SYNC_INTERVAL: Duration = Duration::from_secs(30);

/// 同步状态（供 /api/status 查询）
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SyncStatus {
    /// 最近一次同步完成时间（RFC3339，本地时区）
    pub last_sync: Option<String>,
    /// 最近一次同步的首个错误信息
    pub last_error: Option<String>,
    /// 待同步（dirty）行数
    pub pending: usize,
}

#[derive(Default)]
pub struct SyncOutcome {
    pub pushed: usize,
    pub failed: usize,
    pub error: Option<String>,
}

enum PushResult {
    Created(String),
    Updated,
    Archived,
}

/// 判断是否为「祖先已归档」错误（Notion 会连带归档子孙页面，此时子孙无需再归档）。
/// 注意必须查整个错误链：anyhow 的 `to_string()` 只返回最外层 context。
fn is_archived_ancestor_err(err: &anyhow::Error) -> bool {
    err.chain()
        .any(|c| c.to_string().contains("archived ancestor"))
}

/// 归档页面；祖先已归档导致子孙被连带归档时视为成功，避免同步死循环
async fn archive_tolerant(client: &NotionClient, page_id: &str) -> Result<()> {
    match client.archive_page(page_id).await {
        Err(e) if is_archived_ancestor_err(&e) => Ok(()),
        other => other,
    }
}

/// 对一行数据执行远端推送
async fn push_event(client: &NotionClient, cfg: &Config, ev: &Event) -> Result<PushResult> {
    if ev.deleted {
        if let Some(pid) = &ev.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &ev.notion_page_id {
        client
            .update_page(pid, &notion::event_properties(ev))
            .await?;
        Ok(PushResult::Updated)
    } else {
        let pid = client
            .create_page(&cfg.events_db_id, &notion::event_properties(ev))
            .await?;
        Ok(PushResult::Created(pid))
    }
}

async fn push_expense(client: &NotionClient, cfg: &Config, ex: &Expense) -> Result<PushResult> {
    if ex.deleted {
        if let Some(pid) = &ex.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &ex.notion_page_id {
        client
            .update_page(pid, &notion::expense_properties(ex))
            .await?;
        Ok(PushResult::Updated)
    } else {
        let pid = client
            .create_page(&cfg.expenses_db_id, &notion::expense_properties(ex))
            .await?;
        Ok(PushResult::Created(pid))
    }
}

async fn push_project(client: &NotionClient, cfg: &Config, p: &Project) -> Result<PushResult> {
    if p.deleted {
        if let Some(pid) = &p.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &p.notion_page_id {
        client
            .update_page(pid, &notion::project_properties(p))
            .await?;
        Ok(PushResult::Updated)
    } else {
        let pid = client
            .create_page(&cfg.projects_db_id, &notion::project_properties(p))
            .await?;
        Ok(PushResult::Created(pid))
    }
}

/// 好想法：镜像为「好想法」database 中的页面
async fn push_idea(client: &NotionClient, db_id: &str, idea: &Idea) -> Result<PushResult> {
    if idea.deleted {
        if let Some(pid) = &idea.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &idea.notion_page_id {
        client
            .update_page(pid, &notion::idea_properties(idea))
            .await?;
        Ok(PushResult::Updated)
    } else {
        let pid = client
            .create_page(db_id, &notion::idea_properties(idea))
            .await?;
        Ok(PushResult::Created(pid))
    }
}

/// 今日任务：镜像为「今日任务」database 中的页面
async fn push_task(
    client: &NotionClient,
    db_id: &str,
    task: &Task,
    project_name: Option<&str>,
) -> Result<PushResult> {
    if task.deleted {
        if let Some(pid) = &task.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &task.notion_page_id {
        client
            .update_page(pid, &notion::task_properties(task, project_name))
            .await?;
        Ok(PushResult::Updated)
    } else {
        let pid = client
            .create_page(db_id, &notion::task_properties(task, project_name))
            .await?;
        Ok(PushResult::Created(pid))
    }
}

/// 知识库条目：镜像为「📚 知识库」database 中的行（page）。
/// notes_db_id 为知识库 database id；parent_notion_id 为父笔记的 Notion page id
///（根级笔记为 None，relation 置空）。文档正文以本地为准全量替换 block。
async fn push_note(
    client: &NotionClient,
    note: &Note,
    notes_db_id: &str,
    parent_notion_id: Option<&str>,
) -> Result<PushResult> {
    if note.deleted {
        if let Some(pid) = &note.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &note.notion_page_id {
        client
            .update_page(pid, &notion::note_properties(&note.title, note.kind, parent_notion_id))
            .await?;
        // 文档内容以本地为准全量替换（doc 不会有子页面，删 block 是安全的）
        if note.kind == NoteKind::Doc {
            client
                .replace_children(pid, notion::md_to_blocks(&note.content_md))
                .await?;
        }
        Ok(PushResult::Updated)
    } else {
        // 新建行：parent 为 database_id；properties 含标题/类型/父级 relation
        let pid = client
            .create_page(notes_db_id, &notion::note_properties(&note.title, note.kind, parent_notion_id))
            .await?;
        if note.kind == NoteKind::Doc && !note.content_md.is_empty() {
            client
                .append_children(&pid, notion::md_to_blocks(&note.content_md))
                .await?;
        }
        Ok(PushResult::Created(pid))
    }
}

/// 外部记录：镜像为命名空间根页下的子页面
async fn push_ext_record(
    client: &NotionClient,
    rec: &ExtRecord,
    ns_page_id: &str,
) -> Result<PushResult> {
    if rec.deleted {
        if let Some(pid) = &rec.notion_page_id {
            archive_tolerant(client, pid).await?;
        }
        Ok(PushResult::Archived)
    } else if let Some(pid) = &rec.notion_page_id {
        client
            .update_page(pid, &notion::note_title_properties(&rec.title))
            .await?;
        client
            .replace_children(
                pid,
                notion::ext_record_blocks(&rec.props_json, &rec.content_md),
            )
            .await?;
        Ok(PushResult::Updated)
    } else {
        let pid = client.create_child_page(ns_page_id, &rec.title).await?;
        let blocks = notion::ext_record_blocks(&rec.props_json, &rec.content_md);
        if !blocks.is_empty() {
            client.append_children(&pid, blocks).await?;
        }
        Ok(PushResult::Created(pid))
    }
}

/// 条目在树中的深度（根为 0），用于「父先子后」排序
fn note_depth(note: &Note, all: &HashMap<String, Note>) -> usize {
    let mut depth = 0;
    let mut cur = note.parent_id.clone();
    while let Some(pid) = cur {
        depth += 1;
        match all.get(&pid) {
            // depth 上限防御数据异常成环
            Some(p) if depth <= all.len() => cur = p.parent_id.clone(),
            _ => break,
        }
    }
    depth
}

/// 冲刷一轮 dirty 行。锁只在读写 SQLite 时短暂持有，不跨网络请求。
/// 知识库根页面补建后会写回共享配置并持久化，因此 cfg 以共享引用传入。
pub async fn sync_once(
    db: &Arc<Mutex<Db>>,
    client: &NotionClient,
    cfg: &Arc<Mutex<Config>>,
) -> SyncOutcome {
    let mut outcome = SyncOutcome::default();

    let cfg_snap = match cfg.lock() {
        Ok(c) => c.clone(),
        Err(_) => {
            outcome.error = Some("配置锁不可用".to_string());
            return outcome;
        }
    };
    let snapshot = {
        let g = match db.lock() {
            Ok(g) => g,
            Err(_) => {
                outcome.error = Some("数据库锁不可用".to_string());
                return outcome;
            }
        };
        match (
            g.dirty_events(),
            g.dirty_expenses(),
            g.dirty_projects(),
            g.dirty_notes(),
            g.all_notes_any(),
            g.dirty_ext_records(),
            g.ext_namespaces(),
            g.dirty_ideas(),
            g.dirty_tasks(),
        ) {
            (Ok(e), Ok(x), Ok(p), Ok(n), Ok(a), Ok(r), Ok(m), Ok(i), Ok(t)) => {
                (e, x, p, n, a, r, m, i, t)
            }
            _ => {
                outcome.error = Some("读取 dirty 行失败".to_string());
                return outcome;
            }
        }
    };
    let (events, expenses, projects, notes, all_notes, ext_records, ext_namespaces, ideas, tasks) =
        snapshot;

    let record = |outcome: &mut SyncOutcome, r: Result<PushResult>, what: &str| match r {
        Ok(res) => {
            outcome.pushed += 1;
            Some(res)
        }
        Err(e) => {
            outcome.failed += 1;
            if outcome.error.is_none() {
                outcome.error = Some(format!("{what}: {e:#}"));
            }
            None
        }
    };

    for ev in events {
        let r = push_event(client, &cfg_snap, &ev).await;
        if let Some(res) = record(&mut outcome, r, "事件")
            && let Ok(g) = db.lock()
        {
            let _ = match res {
                PushResult::Created(pid) => g.set_event_page_id(&ev.id, &pid),
                PushResult::Updated => g.clear_event_dirty(&ev.id),
                PushResult::Archived => g.purge_event(&ev.id),
            };
        }
    }
    for ex in expenses {
        let r = push_expense(client, &cfg_snap, &ex).await;
        if let Some(res) = record(&mut outcome, r, "消费")
            && let Ok(g) = db.lock()
        {
            let _ = match res {
                PushResult::Created(pid) => g.set_expense_page_id(&ex.id, &pid),
                PushResult::Updated => g.clear_expense_dirty(&ex.id),
                PushResult::Archived => g.purge_expense(&ex.id),
            };
        }
    }
    for p in projects {
        let r = push_project(client, &cfg_snap, &p).await;
        if let Some(res) = record(&mut outcome, r, "项目")
            && let Ok(g) = db.lock()
        {
            let _ = match res {
                PushResult::Created(pid) => g.set_project_page_id(&p.id, &pid),
                PushResult::Updated => g.clear_project_dirty(&p.id),
                PushResult::Archived => g.purge_project(&p.id),
            };
        }
    }

    // 知识库：父先子后。没有 dirty 条目时不触碰远端（避免无谓补建根页面）
    if !notes.is_empty() {
        sync_notes(
            db,
            client,
            cfg,
            &cfg_snap,
            notes,
            all_notes,
            &mut outcome,
            &record,
        )
        .await;
    }

    // 外部记录：按命名空间分组，懒建 ns 根页
    if !ext_records.is_empty() {
        sync_ext(
            db,
            client,
            &cfg_snap,
            ext_records,
            ext_namespaces,
            &mut outcome,
            &record,
        )
        .await;
    }

    // 好想法：没有 dirty 条目时不触碰远端（避免无谓补建 database）
    if !ideas.is_empty() {
        sync_ideas(db, client, cfg, &cfg_snap, ideas, &mut outcome, &record).await;
    }

    // 今日任务：同 ideas，没有 dirty 条目时不触碰远端
    if !tasks.is_empty() {
        sync_tasks(db, client, cfg, &cfg_snap, tasks, &mut outcome, &record).await;
    }
    outcome
}

/// 好想法同步：config 缺 ideas_db_id 时先补建「好想法」database 并持久化；
/// 建不出则全部记 failed，下轮重试
async fn sync_ideas(
    db: &Arc<Mutex<Db>>,
    client: &NotionClient,
    cfg: &Arc<Mutex<Config>>,
    cfg_snap: &Config,
    ideas: Vec<Idea>,
    outcome: &mut SyncOutcome,
    record: &impl Fn(&mut SyncOutcome, Result<PushResult>, &str) -> Option<PushResult>,
) {
    let db_id = match cfg_snap.ideas_db_id.clone() {
        Some(id) if !id.is_empty() => Some(id),
        _ => match client.create_ideas_db(&cfg_snap.parent_page_id).await {
            Ok(id) => {
                if let Ok(mut c) = cfg.lock() {
                    c.ideas_db_id = Some(id.clone());
                    if let Err(e) = config::save(&c) {
                        outcome.error = Some(format!("好想法: 配置写盘失败: {e:#}"));
                    }
                }
                Some(id)
            }
            Err(e) => {
                outcome.failed += ideas.len();
                if outcome.error.is_none() {
                    outcome.error = Some(format!("好想法: 创建 database 失败: {e:#}"));
                }
                None
            }
        },
    };
    let Some(db_id) = db_id else {
        return;
    };
    for idea in ideas {
        let r = push_idea(client, &db_id, &idea).await;
        if let Some(res) = record(outcome, r, "好想法")
            && let Ok(g) = db.lock()
        {
            let _ = match res {
                PushResult::Created(pid) => g.set_idea_page_id(&idea.id, &pid),
                PushResult::Updated => g.clear_idea_dirty(&idea.id),
                PushResult::Archived => g.purge_idea(&idea.id),
            };
        }
    }
}

/// 今日任务同步：config 缺 tasks_db_id 时先补建「今日任务」database 并持久化；
/// 建不出则全部记 failed，下轮重试
async fn sync_tasks(
    db: &Arc<Mutex<Db>>,
    client: &NotionClient,
    cfg: &Arc<Mutex<Config>>,
    cfg_snap: &Config,
    tasks: Vec<Task>,
    outcome: &mut SyncOutcome,
    record: &impl Fn(&mut SyncOutcome, Result<PushResult>, &str) -> Option<PushResult>,
) {
    let db_id = match cfg_snap.tasks_db_id.clone() {
        Some(id) if !id.is_empty() => Some(id),
        _ => match client.create_tasks_db(&cfg_snap.parent_page_id).await {
            Ok(id) => {
                if let Ok(mut c) = cfg.lock() {
                    c.tasks_db_id = Some(id.clone());
                    if let Err(e) = config::save(&c) {
                        outcome.error = Some(format!("今日任务: 配置写盘失败: {e:#}"));
                    }
                }
                Some(id)
            }
            Err(e) => {
                outcome.failed += tasks.len();
                if outcome.error.is_none() {
                    outcome.error = Some(format!("今日任务: 创建 database 失败: {e:#}"));
                }
                None
            }
        },
    };
    let Some(db_id) = db_id else {
        return;
    };
    // 旧库补新增列（类型/项目/计划开始，幂等；失败不阻断，推送报错会下轮重试）
    if let Err(e) = client
        .update_database(&db_id, &notion::tasks_extra_properties_schema())
        .await
        && outcome.error.is_none()
    {
        outcome.error = Some(format!("今日任务: 更新 database 结构失败: {e:#}"));
    }
    // 项目 id → 名称映射（任务同步「项目」属性用）
    let project_names: HashMap<String, String> = match db.lock() {
        Ok(g) => g
            .all_projects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect(),
        Err(_) => HashMap::new(),
    };
    for task in tasks {
        let pname = task
            .project_id
            .as_ref()
            .and_then(|pid| project_names.get(pid))
            .map(String::as_str);
        let r = push_task(client, &db_id, &task, pname).await;
        if let Some(res) = record(outcome, r, "今日任务")
            && let Ok(g) = db.lock()
        {
            let _ = match res {
                PushResult::Created(pid) => g.set_task_page_id(&task.id, &pid),
                PushResult::Updated => g.clear_task_dirty(&task.id),
                PushResult::Archived => g.purge_task(&task.id),
            };
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn sync_ext(
    db: &Arc<Mutex<Db>>,
    client: &NotionClient,
    cfg_snap: &Config,
    records: Vec<ExtRecord>,
    namespaces: Vec<(String, String)>,
    outcome: &mut SyncOutcome,
    record: &impl Fn(&mut SyncOutcome, Result<PushResult>, &str) -> Option<PushResult>,
) {
    let mut ns_map: HashMap<String, String> = namespaces.into_iter().collect();
    let mut by_ns: HashMap<String, Vec<ExtRecord>> = HashMap::new();
    for r in records {
        by_ns.entry(r.ns.clone()).or_default().push(r);
    }
    for (ns, recs) in by_ns {
        // 懒建命名空间根页「📦 {ns}」
        if !ns_map.contains_key(&ns) {
            match client
                .create_child_page(&cfg_snap.parent_page_id, &format!("📦 {ns}"))
                .await
            {
                Ok(pid) => {
                    if let Ok(g) = db.lock() {
                        let _ = g.set_ext_namespace_page_id(&ns, &pid);
                    }
                    ns_map.insert(ns.clone(), pid);
                }
                Err(e) => {
                    outcome.failed += recs.len();
                    if outcome.error.is_none() {
                        outcome.error =
                            Some(format!("外部记录: 创建命名空间「{ns}」根页面失败: {e:#}"));
                    }
                    continue;
                }
            }
        }
        let ns_page = ns_map[&ns].clone();
        for rec in recs {
            let r = push_ext_record(client, &rec, &ns_page).await;
            if let Some(res) = record(outcome, r, "外部记录")
                && let Ok(g) = db.lock()
            {
                let _ = match res {
                    PushResult::Created(pid) => g.set_ext_record_page_id(&rec.ns, &rec.id, &pid),
                    PushResult::Updated => g.clear_ext_record_dirty(&rec.ns, &rec.id),
                    PushResult::Archived => g.purge_ext_record(&rec.ns, &rec.id),
                };
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn sync_notes(
    db: &Arc<Mutex<Db>>,
    client: &NotionClient,
    cfg: &Arc<Mutex<Config>>,
    cfg_snap: &Config,
    mut notes: Vec<Note>,
    all_notes: Vec<Note>,
    outcome: &mut SyncOutcome,
    record: &impl Fn(&mut SyncOutcome, Result<PushResult>, &str) -> Option<PushResult>,
) {
    // 确保知识库 database 存在；旧配置缺失时补建（含自关联「父级」）并持久化
    let notes_db_id = match cfg_snap.notes_db_id.clone() {
        Some(id) if !id.is_empty() => id,
        _ => match client.create_notes_db(&cfg_snap.parent_page_id).await {
            Ok(id) => {
                if let Ok(mut c) = cfg.lock() {
                    c.notes_db_id = Some(id.clone());
                    if let Err(e) = config::save(&c) {
                        outcome.error = Some(format!("知识库: 配置写盘失败: {e:#}"));
                    }
                }
                id
            }
            Err(e) => {
                outcome.failed += notes.len();
                if outcome.error.is_none() {
                    outcome.error = Some(format!("知识库: 创建数据库失败: {e:#}"));
                }
                return;
            }
        },
    };

    let all_map: HashMap<String, Note> = all_notes.into_iter().map(|n| (n.id.clone(), n)).collect();
    notes.sort_by_key(|n| note_depth(n, &all_map));
    for note in notes {
        // 解析父级 Notion page id（用于 relation）：父 note 的 page id，根级笔记为 None
        let parent_notion_id = match note.parent_id.as_deref() {
            Some(pid) => all_map.get(pid).and_then(|p| p.notion_page_id.clone()),
            None => None,
        };
        // 新建且有父级，但父级尚未同步出 page id -> 本轮跳过（保留 dirty，下轮再来）
        if note.notion_page_id.is_none() && note.parent_id.is_some() && parent_notion_id.is_none() {
            continue;
        }
        let r = push_note(client, &note, &notes_db_id, parent_notion_id.as_deref()).await;
        if let Some(res) = record(outcome, r, "知识库")
            && let Ok(g) = db.lock()
        {
            let _ = match res {
                PushResult::Created(pid) => g.set_note_page_id(&note.id, &pid),
                PushResult::Updated => g.clear_note_dirty(&note.id),
                PushResult::Archived => g.purge_note(&note.id),
            };
        }
    }
}

fn pending_of(db: &Arc<Mutex<Db>>) -> usize {
    db.lock()
        .ok()
        .and_then(|g| g.pending_count().ok())
        .unwrap_or(0)
}

/// 后台同步循环；收到 shutdown 通知后退出。
/// 每轮从共享配置读取最新值：未配置完整时只刷新状态，不访问网络。
pub async fn sync_loop(
    db: Arc<Mutex<Db>>,
    client: Arc<NotionClient>,
    cfg: Arc<Mutex<Config>>,
    shutdown: Arc<Notify>,
    status: Arc<Mutex<SyncStatus>>,
) {
    let mut timer = tokio::time::interval(SYNC_INTERVAL);
    loop {
        tokio::select! {
            _ = timer.tick() => {
                let configured = cfg.lock().map(|c| config::is_configured(&c)).unwrap_or(false);
                let mut last_error = None;
                let mut last_sync = None;
                if configured {
                    let outcome = sync_once(&db, &client, &cfg).await;
                    last_error = outcome.error;
                    last_sync = Some(Local::now().to_rfc3339());
                }
                if let Ok(mut s) = status.lock() {
                    if let Some(t) = last_sync {
                        s.last_sync = Some(t);
                    }
                    s.last_error = last_error;
                    s.pending = pending_of(&db);
                }
            }
            _ = shutdown.notified() => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_archived_ancestor_err;

    #[test]
    fn detects_archived_ancestor_error() {
        let e = anyhow::anyhow!(
            "归档 Notion 页面失败: Notion API 错误 400 Bad Request: \
             {{\"code\":\"validation_error\",\"message\":\"Can't edit page on block \
             with an archived ancestor. You must unarchive the ancestor before editing page.\"}}"
        );
        assert!(is_archived_ancestor_err(&e));

        // 真实场景：anyhow context 包装后 to_string() 只剩外层文案，必须查错误链
        let wrapped = anyhow::anyhow!("Notion API 错误 400: archived ancestor")
            .context("归档 Notion 页面失败");
        assert!(is_archived_ancestor_err(&wrapped));

        let other = anyhow::anyhow!("Notion API 错误 429 Too Many Requests");
        assert!(!is_archived_ancestor_err(&other));
    }
}
