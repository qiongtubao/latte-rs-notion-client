//! Latte 桌面壳（Tauri v2）。
//!
//! 复用 Web 版核心（`latte::server`）：壳进程内启动 axum 本地服务（127.0.0.1:3210），
//! 各窗口 Webview 直接加载该地址，前端与 Web 版完全一致，走同一套 HTTP API。
//!
//! 窗口结构：
//! - `main`：主窗口（标签页界面），默认隐藏，托盘/弹窗可唤起，关闭时仅隐藏；
//! - `ball-<key>`：4 个系统级悬浮球（透明、无边框、置顶、跳过任务栏），可拖拽，位置持久化；
//! - `popup`：共享的快捷操作弹窗，点击悬浮球时在球旁边弹出对应功能视图。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

const SERVER_URL: &str = "http://127.0.0.1:3210";
/// 悬浮球 key 列表（与前端 PopupApp 的 viewMap 一一对应）
const BALL_KEYS: [&str; 4] = ["today", "money", "calendar", "projects"];
/// 弹窗尺寸（逻辑像素）
const POPUP_W: f64 = 440.0;
const POPUP_H: f64 = 560.0;

/// 当前弹窗展示的功能 key（用于同 key 再次点击时关闭）
type PopupKey = Mutex<Option<String>>;

// ---------- 悬浮球位置持久化 ----------

fn positions_path() -> std::path::PathBuf {
    latte::config::config_dir().join("ball_positions.json")
}

fn load_positions() -> HashMap<String, (i32, i32)> {
    std::fs::read_to_string(positions_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_position(key: &str, x: i32, y: i32) {
    let mut map = load_positions();
    map.insert(key.to_string(), (x, y));
    let path = positions_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string(&map) {
        let _ = std::fs::write(&path, text);
    }
}

// ---------- 悬浮球透明开关 ----------

/// 悬浮球是否使用透明窗口。
/// 配置项 `ball_transparent`（~/.config/latte/config.toml）可强制指定；
/// 缺省自动：Wayland / 非 Linux 透明；Linux X11 保守取不透明圆角色块——
/// X11 下无法可靠判断合成器是否真的混合 ARGB（xrdp/软渲染等场景透明窗口退化为白方块）。
fn ball_transparent() -> bool {
    if let Some(forced) = latte::config::load()
        .ok()
        .flatten()
        .and_then(|c| c.ball_transparent)
    {
        return forced;
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

// ---------- Tauri 命令 ----------

#[tauri::command]
fn show_main(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

#[tauri::command]
fn hide_popup(app: tauri::AppHandle) {
    if let Some(p) = app.get_webview_window("popup") {
        let _ = p.hide();
    }
    if let Some(state) = app.try_state::<PopupKey>() {
        if let Ok(mut cur) = state.lock() {
            *cur = None;
        }
    }
}

/// 点击悬浮球：在球旁边弹出/切换/关闭快捷操作弹窗
#[tauri::command]
fn toggle_popup(app: tauri::AppHandle, key: String) -> Result<(), String> {
    let popup = app.get_webview_window("popup").ok_or("popup 窗口不存在")?;
    let ball = app
        .get_webview_window(&format!("ball-{key}"))
        .ok_or_else(|| format!("悬浮球 ball-{key} 不存在"))?;

    {
        let state = app.try_state::<PopupKey>().ok_or("状态未初始化")?;
        let mut cur = state.lock().map_err(|e| e.to_string())?;
        if popup.is_visible().map_err(|e| e.to_string())? && cur.as_deref() == Some(&key) {
            let _ = popup.hide();
            *cur = None;
            return Ok(());
        }
        *cur = Some(key.clone());
    }

    // 位置：球在屏幕右半侧则弹窗放球左边，否则放右边；垂直方向居中并夹在屏幕内
    let pos = ball.outer_position().map_err(|e| e.to_string())?;
    let bsize = ball.outer_size().map_err(|e| e.to_string())?;
    // 优先用球所在的显示器，多屏时不会弹到别的屏幕上
    let monitor = ball
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let (mon_x, mon_y, mon_w, mon_h, sf) = match monitor {
        Some(m) => (
            m.position().x,
            m.position().y,
            m.size().width as i32,
            m.size().height as i32,
            m.scale_factor(),
        ),
        None => (0, 0, 1920, 1080, 1.0),
    };
    let pw = (POPUP_W * sf) as i32;
    let ph = (POPUP_H * sf) as i32;
    let gap = (12.0 * sf) as i32;
    let ball_cx = pos.x + bsize.width as i32 / 2;
    let x = if ball_cx > mon_x + mon_w / 2 {
        pos.x - pw - gap
    } else {
        pos.x + bsize.width as i32 + gap
    };
    let y = (pos.y + bsize.height as i32 / 2 - ph / 2).clamp(mon_y, mon_y + mon_h - ph);

    popup
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    app.emit("popup-set", key).map_err(|e| e.to_string())?;
    popup.show().map_err(|e| e.to_string())?;
    popup.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// 获取当前配置的全局快捷键
#[tauri::command]
fn get_global_shortcut() -> String {
    latte::config::load()
        .ok()
        .flatten()
        .map(|c| c.global_shortcut)
        .unwrap_or_else(|| "Alt+Q".to_string())
}
/// 修改全局快捷键：写 config.toml + 重新注册（先 unregister_all 再 on_shortcut）
#[tauri::command]
fn set_global_shortcut(app: tauri::AppHandle, new_key: String) -> Result<String, String> {
    let key = new_key.trim().to_string();
    if key.is_empty() {
        return Err("快捷键不能为空".into());
    }
    // 写配置文件
    let mut cfg = latte::config::load().ok().flatten().unwrap_or_default();
    cfg.global_shortcut = key.clone();
    latte::config::save(&cfg).map_err(|e| e.to_string())?;
    // 重新注册：先全部注销，再注册新的
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let _ = app.global_shortcut().unregister_all();
    use tauri_plugin_global_shortcut::{ShortcutEvent, ShortcutState};
    use tauri::Emitter;
    let _ = app.global_shortcut().on_shortcut(
        key.as_str(),
        move |app, _shortcut, event: ShortcutEvent| {
            if event.state == ShortcutState::Pressed {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
                let _ = app.emit("latte-global-quick-entry", ());
            }
        },
    );
    Ok(key)
}

 // ---------- 窗口创建 ----------

/// 创建 5 个悬浮球窗口和 1 个共享弹窗（需主线程调用，服务就绪后进行）
fn create_floating_windows(app: &tauri::AppHandle) {
    let positions = load_positions();
    let (mon_x, mon_y, mon_w, sf) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.scale_factor()))
        .unwrap_or((0, 0, 1920, 1.0));

    let transparent = ball_transparent();
    for (i, key) in BALL_KEYS.iter().enumerate() {
        // 默认位置：屏幕右边缘竖排
        let default = (
            mon_x + mon_w - (90.0 * sf) as i32,
            mon_y + (160.0 * sf) as i32 + (i as f64 * 84.0 * sf) as i32,
        );
        let (x, y) = positions.get(*key).copied().unwrap_or(default);
        let url = if transparent {
            format!("{SERVER_URL}/?ball={key}")
        } else {
            format!("{SERVER_URL}/?ball={key}&opaque=1")
        };
        let win = match WebviewWindowBuilder::new(app, format!("ball-{key}"), WebviewUrl::External(url.parse().unwrap()))
            .title("Latte")
            .transparent(transparent)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .focused(false)
            .inner_size(64.0, 64.0)
            .build()
        {
            Ok(w) => w,
            Err(e) => {
                eprintln!("创建悬浮球 ball-{key} 失败：{e:?}");
                continue;
            }
        };
        // saved/default 位置均为物理坐标
        let _ = win.set_position(PhysicalPosition::new(x, y));
        // 拖拽移动后持久化位置（Moved 事件对程序内 set_position 也会触发，重写相同值无害）
        let key_owned = key.to_string();
        win.on_window_event(move |event| {
            if let WindowEvent::Moved(pos) = event {
                save_position(&key_owned, pos.x, pos.y);
            }
        });
    }

    if let Err(e) = WebviewWindowBuilder::new(
        app,
        "popup",
        WebviewUrl::External(format!("{SERVER_URL}/?popup=1").parse().unwrap()),
    )
    .title("Latte")
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .inner_size(POPUP_W, POPUP_H)
    .build()
    {
        eprintln!("创建弹窗失败：{e:?}");
    }
}

/// 桌面悬浮球 UI 总开关。
/// 悬浮球窗口样式由 `ball_transparent()` 决定：Wayland/macOS 透明圆球，
/// Linux X11 默认不透明圆角色块（可在 config.toml 设 `ball_transparent = true` 强制透明）。
/// 托盘与「关闭仅隐藏」不受此影响——它们一直启用，保证窗口关掉后提醒仍然存活。
const FLOATING_UI_ENABLED: bool = true;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(PopupKey::new(None))
        .setup(|app| {
            // 全局快捷键：Alt+Q 呼出主窗口并唤起快速录入（即使窗口未聚焦/已隐藏）
            use tauri_plugin_global_shortcut::{ShortcutEvent, ShortcutState};
            use tauri::Emitter;
            // 全局快捷键：从 config.toml 读取，缺省 Alt+Q
            let initial_key = latte::config::load()
                .ok()
                .flatten()
                .map(|c| c.global_shortcut)
                .unwrap_or_else(|| "Alt+Q".to_string());
            let _ = app.handle().global_shortcut().on_shortcut(
                initial_key.as_str(),
                move |app, _shortcut, event: ShortcutEvent| {
                    if event.state == ShortcutState::Pressed {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                        let _ = app.emit("latte-global-quick-entry", ());
                    }
                },
            );
            if let Some(main) = app.get_webview_window("main") {
                let w = main.clone();
                main.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = w.hide();
                    }
                });
            }

            // 系统托盘：打开主窗口 / 退出
            let show = MenuItem::with_id(app, "show", "打开主窗口", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 Latte", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("Latte");
            match app.default_window_icon() {
                Some(icon) => tray = tray.icon(icon.clone()),
                None => {
                    if let Ok(img) = Image::from_bytes(include_bytes!("../icons/32x32.png")) {
                        tray = tray.icon(img);
                    }
                }
            }
            tray.on_menu_event(|app, event| match event.id().as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.show();
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;

            // 壳内启动 latte 本地服务；悬浮球 UI 开启时在服务就绪后创建悬浮球窗口
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let shutdown = Arc::new(tokio::sync::Notify::new());
                match latte::server::init_server(shutdown.clone()).await {
                    Ok((_state, _serve)) => {
                        // 服务就绪；init_server 内部已 spawn serve，这里无需再持有
                        drop(_serve);
                    }
                    Err(e) => {
                        // 端口被占：可能已有实例在跑，直接复用该地址
                        eprintln!("Latte 服务启动失败：{e:?}");
                    }
                }
                if FLOATING_UI_ENABLED {
                    // 窗口创建必须在主线程
                    let h2 = handle.clone();
                    let _ = handle.run_on_main_thread(move || create_floating_windows(&h2));
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![show_main, hide_popup, toggle_popup, get_global_shortcut, set_global_shortcut])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
