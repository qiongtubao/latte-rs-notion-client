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

/// 构建共享应用状态（DB / Notion 客户端 / 配置 / 同步状态）。
/// Web 入口与 Tauri 壳都从这里拿 state。
pub fn build_state(
    cfg: config::Config,
    db_path: &std::path::Path,
) -> Result<AppState> {
    let state = AppState {
        db: Arc::new(Mutex::new(
            Db::open(db_path).context("打开本地数据库失败")?,
        )),
        client: Arc::new(NotionClient::new(cfg.token.clone())),
        config: Arc::new(Mutex::new(cfg)),
        sync_status: Arc::new(Mutex::new(SyncStatus::default())),
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
