//! Latte 桌面壳（Tauri v2）。
//!
//! 复用 Web 版核心（`latte::server`）：壳进程内启动 axum 本地服务（127.0.0.1:3210），
//! Webview 直接加载该地址（见 tauri.conf.json 的 window.url）。
//! 前端与 Web 版完全一致，走同一套 HTTP API。

use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            // 壳内启动 latte 本地服务
            tauri::async_runtime::spawn(async move {
                let shutdown = Arc::new(tokio::sync::Notify::new());
                match latte::server::init_server(shutdown.clone()).await {
                    Ok((_state, _serve)) => {
                        // 服务就绪；窗口 URL 已在 tauri.conf.json 配置为 http://127.0.0.1:3210
                        // init_server 内部已 spawn serve，这里无需再持有
                        drop(_serve);
                    }
                    Err(e) => {
                        // 端口被占：可能已有实例在跑，Windows 默认加载该地址仍可用
                        eprintln!("Latte 服务启动失败：{e:?}");
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
