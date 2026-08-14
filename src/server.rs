//! HTTP 服务装配：构建共享状态、组装路由、启动 axum 服务。
//!
//! 同时服务于 Web（main.rs）与桌面（Tauri 壳）：桌面版调用本模块构建状态后，
//! 既可通过 axum 提供 HTTP API，也可复用它暴露 Tauri commands。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::api::{self, AppState};
use crate::config;
use crate::db::Db;
use crate::notion::NotionClient;
use crate::sync::{SyncStatus, sync_loop};
use tokio::sync::Notify;

/// HTTP 服务监听的本地地址（Web 与桌面共用）
pub const DEFAULT_ADDR: &str = "127.0.0.1:3210";

/// 最多保留的自动备份份数
const DB_BACKUP_KEEP: usize = 7;

/// 启动时备份本地数据库：复制为 `latte.db.bak-YYYYMMDD`（每天最多一份），只留最近 7 份。
/// 在 DB 打开后、任何写入前调用，此时直接拷贝文件是安全的。
fn backup_db_file(db_path: &std::path::Path) {
    let Some(dir) = db_path.parent() else { return };
    if !db_path.is_file() {
        return; // 首次运行还没有数据可备份
    }
    let stamp = chrono::Local::now().format("%Y%m%d");
    let dest = dir.join(format!("latte.db.bak-{stamp}"));
    if !dest.exists()
        && let Err(e) = std::fs::copy(db_path, &dest)
    {
        eprintln!("备份数据库失败：{e:?}");
    }
    // 超出保留份数的最老备份删掉（文件名带日期，字典序即时间序）
    let mut backups: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.starts_with("latte.db.bak-"))
                .collect()
        })
        .unwrap_or_default();
    backups.sort();
    for name in backups.iter().take(backups.len().saturating_sub(DB_BACKUP_KEEP)) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// 构建共享应用状态（DB / Notion 客户端 / 配置 / 同步状态）。
/// Web 入口与 Tauri 壳都从这里拿 state。
pub fn build_state(
    cfg: config::Config,
    db_path: &std::path::Path,
) -> Result<AppState> {
    let db = Db::open(db_path).context("打开本地数据库失败")?;
    // 重建知识库全文索引（兜底漏钩子的历史数据；表小，重建开销可忽略）
    db.rebuild_notes_fts().context("重建知识库检索索引失败")?;
    if let Ok(notes) = db.all_notes() {
        crate::docgraph::export_notes_logged(&notes);
    }
    let state = AppState {
        db: Arc::new(Mutex::new(db)),
        client: Arc::new(NotionClient::new(cfg.token.clone())),
        config: Arc::new(Mutex::new(cfg)),
        sync_status: Arc::new(Mutex::new(SyncStatus::default())),
        pomodoro: Arc::new(Mutex::new(None)),
    };
    Ok(state)
}

/// 组装完整 router（API + CORS + 可选的 SPA 静态回退）。
pub fn build_router(state: AppState) -> Router {
    let app = api::router(state.clone()).layer(CorsLayer::permissive());
    match static_dir() {
        // SPA：静态文件未命中时回退到 index.html
        Some(dir) => app
            .fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => {
            println!("提示：未找到 frontend/dist，仅提供 /api 服务。");
            app
        }
    }
}

/// 前端构建产物目录：crate 根（开发期）→ 可执行文件同级 → 当前目录下的 frontend/dist
fn static_dir() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frontend/dist")];
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("frontend/dist"));
    }
    candidates.push(PathBuf::from("frontend/dist"));
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

/// 启动 axum 服务，返回后台 task handle 与共享状态。
///
/// 完整生命周期（构建状态 → 同步循环 → serve → 关闭冲刷）由调用方编排：
/// - Web 入口（main.rs）：await handle，等待 Ctrl-C
/// - 桌面壳（Tauri）：持有 state 暴露 commands，需要关闭时 notify + await handle
pub async fn init_server(
    shutdown: Arc<Notify>,
) -> Result<(AppState, tokio::task::JoinHandle<()>)> {
    let mut cfg = config::load()?.unwrap_or_default();
    if config::is_configured(&cfg) && cfg.api_token.is_empty() {
        cfg.api_token = uuid::Uuid::new_v4().to_string();
        config::save(&cfg)?;
        println!("外部 API token 已生成，见 config.toml");
    }
    let state = build_state(cfg, &config::db_path())?;
    backup_db_file(&config::db_path());

    // 后台同步循环
    let sync_state = state.clone();
    let sync_shutdown = shutdown.clone();
    let _sync_handle = tokio::spawn(async move {
        sync_loop(
            sync_state.db.clone(),
            sync_state.client.clone(),
            sync_state.config.clone(),
            sync_shutdown.clone(),
            sync_state.sync_status.clone(),
        )
        .await;
    });

    // 到点提醒循环：系统级通知（Linux D-Bus），浏览器/窗口关了也有效
    let rem_state = state.clone();
    let rem_shutdown = shutdown.clone();
    tokio::spawn(async move {
        reminder_loop(rem_state, rem_shutdown).await;
    });

    // 启动后自动从 Notion 拉取一次（等首轮推送完成；仍有待同步数据则放弃，避免覆盖）
    let pull_state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        auto_pull_once(pull_state).await;
    });

    // 组装 router 并 serve
    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(DEFAULT_ADDR)
        .await
        .with_context(|| format!("监听 {DEFAULT_ADDR} 失败"))?;
    println!("Latte 已启动：http://{DEFAULT_ADDR}");

    let serve = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("服务异常退出：{e:?}");
        }
    });

    Ok((state, serve))
}

/// 启动后自动拉取一次远端数据：只在本地没有未同步（dirty）行时执行，
/// 否则放弃并留给用户手动「拉取」，避免全量覆盖冲掉未推送的本地修改。
async fn auto_pull_once(state: AppState) {
    let cfg = state.config.lock().map(|c| c.clone()).unwrap_or_default();
    if !config::is_configured(&cfg) {
        return;
    }
    let pending = state
        .db
        .lock()
        .ok()
        .and_then(|db| db.pending_count().ok())
        .unwrap_or(1);
    if pending > 0 {
        println!("启动自动拉取跳过：有 {pending} 条未同步数据");
        return;
    }
    match state.client.pull_all(&cfg).await {
        Ok(data) => {
            let res = state.db.lock().ok().map(|db| db.pull_replace_all(&data));
            match res {
                Some(Ok(c)) => println!(
                    "启动自动拉取完成：事件 {} · 消费 {} · 项目 {} · 想法 {} · 任务 {} · 知识库 {}",
                    c.events, c.expenses, c.projects, c.ideas, c.tasks, c.notes
                ),
                Some(Err(e)) => eprintln!("启动自动拉取写库失败：{e:#}"),
                None => eprintln!("启动自动拉取失败：数据库锁不可用"),
            }
        }
        Err(e) => eprintln!("启动自动拉取失败：{e:#}"),
    }
}

/// 到点提醒循环：每 30s 把 start_ts 落在上一检查点至今的 remind 事件发系统通知。
///
/// 水位线只存内存：进程启动前的过期事件不补发，重启不会轰炸旧提醒。
/// 通知失败（如无 D-Bus 通知守护进程）只打日志，不影响主流程。
async fn reminder_loop(state: AppState, shutdown: Arc<Notify>) {
    let mut last = chrono::Local::now().timestamp();
    let mut last_review: Option<chrono::NaiveDate> = None;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {}
            _ = shutdown.notified() => break,
        }
        let now = chrono::Local::now().timestamp();
        let enabled = state.config.lock().map(|c| c.remind_enabled).unwrap_or(false);
        if enabled {
            let due = state
                .db
                .lock()
                .ok()
                .and_then(|db| db.due_reminders(last, now).ok())
                .unwrap_or_default();
            for ev in due {
                notify("Latte 提醒", &format!(
                    "该开始了：{}（{}）",
                    if ev.content.is_empty() { "(无内容)" } else { &ev.content },
                    ev.tag.label()
                ));
            }
            // 每日打卡项提醒（按 HH:MM + remind_days 过滤，每分钟最多 1 次）
            let due_items = state
                .db
                .lock()
                .ok()
                .and_then(|db| {
                    let now_h = chrono::Local::now();
                    let hhmm = now_h.format("%H:%M").to_string();
                    db.daily_items_due_at(&hhmm).ok()
                })
                .unwrap_or_default();
            for it in due_items {
                notify(
                    "Latte 打卡",
                    &format!("🍅 该打卡了：{}{}", it.name, if it.unit.is_empty() { String::new() } else { format!("（{}）", it.unit) }),
                );
            }
            // 番茄钟到点：通知后清除会话（关联事件继续计时，由用户手动结束）
            let pomo_done = state
                .pomodoro
                .lock()
                .ok()
                .and_then(|mut p| p.take_if(|s| s.end_ts <= now));
            if let Some(s) = pomo_done {
                notify("Latte 番茄钟", &format!("🍅 {} 分钟到：{}，休息一下", s.minutes, s.task_title));
            }
            // 每日复盘：21:30 后推一次当天汇总（内存记日期当天只发一次；23:30 后不补发）
            let today = chrono::Local::now().date_naive();
            let review_ts = crate::report::local_midnight(today) + 21 * 3600 + 30 * 60;
            if last_review != Some(today) && now >= review_ts && now < review_ts + 2 * 3600 {
                last_review = Some(today);
                notify("Latte 每日复盘", &build_daily_review(&state, today, now));
            }
        }
        last = now;
    }
}

/// 拼当天复盘文案：记录段数与总时长（前 3 个标签）· 消费 · 完成任务数
fn build_daily_review(state: &AppState, today: chrono::NaiveDate, now: i64) -> String {
    let from = crate::report::local_midnight(today);
    let Ok(db) = state.db.lock() else {
        return "读取本地数据失败".to_string();
    };
    let events = db.events_between(from, now, now).unwrap_or_default();
    let (tags, total) = crate::report::summarize_events(&events, from, now, now);
    let cents: i64 = db
        .expenses_between(from, now)
        .unwrap_or_default()
        .iter()
        .map(|e| e.amount_cents)
        .sum();
    let done = db
        .tasks_on(&today.to_string())
        .map(|ts| ts.iter().filter(|t| t.done).count())
        .unwrap_or(0);
    let mut parts = Vec::new();
    if total > 0 {
        let tag_text = tags
            .iter()
            .take(3)
            .map(|t| format!("{} {}", t.tag.label(), crate::report::fmt_duration(t.seconds)))
            .collect::<Vec<_>>()
            .join(" / ");
        parts.push(format!(
            "记录 {} 段共 {}（{tag_text}）",
            events.len(),
            crate::report::fmt_duration(total)
        ));
    } else {
        parts.push("今天还没有时间记录".to_string());
    }
    parts.push(format!("消费 ¥{:.2}", cents as f64 / 100.0));
    parts.push(format!("完成任务 {done}"));
    parts.join(" · ")
}

/// 发系统通知（Linux D-Bus）；show() 是阻塞调用，放进 blocking 池，失败只打日志
fn notify(summary: &str, body: &str) {
    let summary = summary.to_string();
    let body = body.to_string();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = notify_rust::Notification::new()
            .summary(&summary)
            .body(&body)
            .show()
        {
            eprintln!("发送系统通知失败：{e:?}");
        }
    });
}

/// 同步循环退出后的最后冲刷：把仍在 dirty 的记录推送一次，然后关闭 DB。
pub async fn shutdown_and_flush(state: &AppState) {
    let configured = state
        .config
        .lock()
        .map(|c| config::is_configured(&c))
        .unwrap_or(false);
    if configured {
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::sync::sync_once(&state.db, &state.client, &state.config),
        )
        .await;
        match outcome {
            Ok(o) if o.failed > 0 => {
                eprintln!("注意：有 {} 条记录未能同步，将在下次启动时重试。", o.failed);
            }
            Err(_) => eprintln!("注意：最后同步超时，未同步的记录将在下次启动时重试。"),
            _ => {}
        }
    }
}
