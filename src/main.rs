//! Latte — 本地 Notion 客户端（Web 版入口）。
//!
//! 装配逻辑见 [`latte::server`]，此处只需：启动服务 → 等 Ctrl-C → 最后冲刷。

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use latte::server::{self, shutdown_and_flush};
use tokio::sync::Notify;

#[tokio::main]
async fn main() -> Result<()> {
    let shutdown = Arc::new(Notify::new());

    let (state, serve) = server::init_server(shutdown.clone()).await?;

    // 等待 Ctrl-C
    let _ = tokio::signal::ctrl_c().await;

    // 停止后台同步循环，给它 2 秒收尾
    shutdown.notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(2), serve).await;

    // 最后一次冲刷
    shutdown_and_flush(&state).await;
    Ok(())
}
