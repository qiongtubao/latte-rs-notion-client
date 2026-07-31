//! Latte — 本地 Notion 客户端（时间碎片 / 金钱 / 日历 / 项目）
//!
//! 入口：加载配置 → 初始化 SQLite → 启动 axum Web 服务（127.0.0.1:3210）
//! + 后台同步循环 → 收到 Ctrl-C 后停止服务并最后冲刷一次。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use latte::api::{self, AppState};
use latte::config;
use latte::db::Db;
use latte::notion::NotionClient;
use latte::sync::{self, SyncStatus};
use tokio::sync::Notify;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

const ADDR: &str = "127.0.0.1:3210";

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

#[tokio::main]
async fn main() -> Result<()> {
    let mut cfg = config::load()?.unwrap_or_default();
    // 已完成 Notion 配置但还没有外部 API token 时自动生成一个
    if config::is_configured(&cfg) && cfg.api_token.is_empty() {
        cfg.api_token = uuid::Uuid::new_v4().to_string();
        config::save(&cfg)?;
        println!("外部 API token 已生成，见 config.toml");
    }
    let configured_at_start = config::is_configured(&cfg);
    let state = AppState {
        db: Arc::new(Mutex::new(
            Db::open(&config::db_path()).context("打开本地数据库失败")?,
        )),
        client: Arc::new(NotionClient::new(cfg.token.clone())),
        config: Arc::new(Mutex::new(cfg)),
        sync_status: Arc::new(Mutex::new(SyncStatus::default())),
    };

    let shutdown = Arc::new(Notify::new());
    let sync_handle = tokio::spawn(sync::sync_loop(
        state.db.clone(),
        state.client.clone(),
        state.config.clone(),
        shutdown.clone(),
        state.sync_status.clone(),
    ));

    let app = api::router(state.clone()).layer(CorsLayer::permissive());
    let app = match static_dir() {
        // SPA：静态文件未命中时回退到 index.html（fallback 保留 ServeFile 的 200；
        // not_found_service 会强制 404，不适合 SPA）
        Some(dir) => app
            .fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")))),
        None => {
            println!("提示：未找到 frontend/dist，仅提供 /api 服务。");
            app
        }
    };

    let listener = tokio::net::TcpListener::bind(ADDR)
        .await
        .with_context(|| format!("监听 {ADDR} 失败"))?;
    println!("Latte 已启动：http://{ADDR}");
    if !configured_at_start {
        println!("尚未完成 Notion 配置，请打开页面进行初始化设置。");
    }

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;

    // 退出：停止后台循环，最后冲刷一次（最多 5 秒）
    shutdown.notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(2), sync_handle).await;
    let configured = state
        .config
        .lock()
        .map(|c| config::is_configured(&c))
        .unwrap_or(false);
    if configured {
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            sync::sync_once(&state.db, &state.client, &state.config),
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
    Ok(())
}
