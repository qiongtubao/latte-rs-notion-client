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
/// 悬浮球直径（物理像素）；逻辑尺寸按显示器缩放系数换算，保证各环境显示一致
const BALL_PHYSICAL_PX: f64 = 56.0;

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

// ---------- X11 圆形窗口裁剪（无合成器环境的圆球方案） ----------

/// X11 下把本进程的悬浮球窗口裁剪成正圆形（显示与输入同时裁剪），不依赖合成器。
/// 原理：X Shape 扩展给窗口套圆形 bounding mask；用「进程 PID + 正方形小窗」
/// 反查球窗 XID（球是窗口里唯一正方形的小窗），避免引入 gtk 依赖。
/// 注意：需在窗口实际映射后调用（创建后立刻调用可能匹配不到/拿到未布局尺寸）。
#[cfg(target_os = "linux")]
fn apply_round_shape() {
    use x11rb::connection::Connection;
    use x11rb::protocol::shape::{self, SK, SO};
    use x11rb::protocol::xproto::{AtomEnum, ChangeGCAux, ConnectionExt, CreateGCAux};

    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    let Some(pid_atom) = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
    else {
        return;
    };
    let my_pid = std::process::id();
    let Ok(Ok(tree)) = conn.query_tree(root).map(|c| c.reply()) else {
        return;
    };
    let mut targets: Vec<(u32, u16)> = Vec::new(); // (xid, 边长)
    for win in tree.children {
        let pid_match = conn
            .get_property(false, win, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut it| it.next()))
            == Some(my_pid);
        if !pid_match {
            continue;
        }
        let geo = conn.get_geometry(win).ok().and_then(|c| c.reply().ok());
        if let Some(g) = geo {
            // 悬浮球特征：正方形小窗（32..=1024 物理像素）。
            // 注意 xrdp/mutter 会给无边框窗口强加 200x200 最小尺寸（任何途径都改不小），
            // 所以不缩窗，只把可见/可点区域裁成居中的小圆
            if g.width == g.height && (32..=1024).contains(&g.width) {
                targets.push((win, g.width));
            }
        }
    }

    for (win, size) in targets {
        // 1 位深 pixmap 画实心圆作 mask（直径 = 目标球径，窗口内居中），套到窗口 bounding shape
        let r = (|| -> Result<(), Box<dyn std::error::Error>> {
            let side = BALL_PHYSICAL_PX as u16;
            let off = size.saturating_sub(side) / 2;
            let pixmap = conn.generate_id()?;
            conn.create_pixmap(1, pixmap, win, size, size)?;
            let gc = conn.generate_id()?;
            conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(0))?;
            conn.poly_fill_rectangle(
                pixmap,
                gc,
                &[x11rb::protocol::xproto::Rectangle {
                    x: 0,
                    y: 0,
                    width: size,
                    height: size,
                }],
            )?;
            conn.change_gc(gc, &ChangeGCAux::new().foreground(1))?;
            conn.poly_fill_arc(
                pixmap,
                gc,
                &[x11rb::protocol::xproto::Arc {
                    x: off as i16,
                    y: off as i16,
                    width: side,
                    height: side,
                    angle1: 0,
                    angle2: 360 * 64,
                }],
            )?;
            shape::mask(&conn, SO::SET, SK::BOUNDING, win, 0, 0, pixmap)?;
            conn.free_gc(gc)?;
            conn.free_pixmap(pixmap)?;
            // RustConnection 会缓冲写请求，不显式 flush 请求可能永远不发出去
            conn.flush()?;
            Ok(())
        })();
        if let Err(e) = r {
            eprintln!("悬浮球圆形裁剪失败：{e}");
        }
    }
}

/// 非 Linux 平台无需圆形裁剪（走透明窗口）
#[cfg(not(target_os = "linux"))]
fn apply_round_shape() {}

// ---------- X11 悬浮球原生输入接管 ----------
//
// xrdp 等残缺 X11 环境下，wry 嵌入的 webview 收不到鼠标按键事件
// （motion 正常、button 丢失，Tauri v1/v2 均中招，属环境级问题）。
// 悬浮球仅有的两个交互——点击弹弹窗、拖拽换位——改由壳在 X11 层直接接管：
// 监听 root 的 XI2 raw 事件，命中球窗几何则调用对应逻辑；
// 球窗 URL 带 native_input=1，前端 BallApp 不再处理鼠标输入（避免健康环境下双重触发）。

/// 是否启用悬浮球原生输入接管：Linux X11 会话（非 Wayland）
#[cfg(target_os = "linux")]
fn native_ball_input_enabled() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_none()
        && !std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
        && std::env::var_os("DISPLAY").is_some()
}

#[cfg(target_os = "linux")]
fn start_native_ball_input(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = native_ball_input_loop(app) {
            eprintln!("悬浮球原生输入监听退出：{e}");
        }
    });
}

#[cfg(target_os = "linux")]
fn native_ball_input_loop(app: tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xinput::{self};
    use x11rb::protocol::xproto::ConnectionExt as _;

    fn fp(v: i32) -> f64 {
        v as f64 / 65536.0
    }

    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    xinput::xi_query_version(&conn, 2, 2)?.reply()?;
    xinput::xi_select_events(
        &conn,
        root,
        &[xinput::EventMask {
            deviceid: xinput::Device::ALL_MASTER.into(),
            mask: vec![xinput::XIEventMask::RAW_BUTTON_PRESS
                | xinput::XIEventMask::RAW_BUTTON_RELEASE
                | xinput::XIEventMask::RAW_MOTION],
        }],
    )?;
    conn.flush()?;

    let pid_atom = conn.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
    let name_atom = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
    let my_pid = std::process::id();
    const DRAG_THRESHOLD: f64 = 8.0;
    // 左键按下状态：Some((命中的球 key[可空], 按下 x, y, 是否已转为拖拽))
    let mut pressed: Option<(Option<String>, f64, f64, bool)> = None;

    loop {
        match conn.wait_for_event()? {
            Event::XinputRawButtonPress(e) if e.detail == 1 => {
                let pt = xinput::xi_query_pointer(&conn, root, e.deviceid)?.reply()?;
                let (px, py) = (fp(pt.root_x), fp(pt.root_y));
                let key = hit_ball(&conn, root, pid_atom, name_atom, my_pid, px, py);
                pressed = Some((key, px, py, false));
            }
            Event::XinputRawMotion(e) => {
                if let Some((Some(key), x0, y0, false)) = pressed.clone() {
                    let pt = xinput::xi_query_pointer(&conn, root, e.deviceid)?.reply()?;
                    let (px, py) = (fp(pt.root_x), fp(pt.root_y));
                    if (px - x0).hypot(py - y0) > DRAG_THRESHOLD {
                        pressed = Some((Some(key.clone()), x0, y0, true));
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            if let Some(w) = app2.get_webview_window(&format!("ball-{key}")) {
                                let _ = w.start_dragging();
                            }
                        });
                    }
                }
            }
            Event::XinputRawButtonRelease(e) if e.detail == 1 => {
                let Some((key, x0, y0, dragged)) = pressed.take() else {
                    continue;
                };
                if dragged {
                    continue;
                }
                match key {
                    // 球上松开：切换弹窗
                    Some(key) => {
                        let app2 = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            let _ = toggle_popup(app2, key);
                        });
                    }
                    // 弹窗头部操作区松开（按下点也在同一区内才算点击）：
                    // 右上角 = 关闭，其次 = 打开主窗口
                    None => {
                        let pt = xinput::xi_query_pointer(&conn, root, e.deviceid)?.reply()?;
                        let (px, py) = (fp(pt.root_x), fp(pt.root_y));
                        let zone_at = |qx: f64, qy: f64| {
                            hit_popup_chrome(&conn, root, pid_atom, name_atom, my_pid, qx, qy)
                        };
                        match (zone_at(x0, y0), zone_at(px, py)) {
                            (Some(a), Some(b)) if a == b => {
                                let app2 = app.clone();
                                let _ = app.run_on_main_thread(move || match a {
                                    "close" => hide_popup(app2),
                                    _ => show_main(app2),
                                });
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// 命中测试：返回 (x, y) 处的悬浮球 key（球窗标题为 latte-ball-<key>）
#[cfg(target_os = "linux")]
fn hit_ball(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    pid_atom: u32,
    name_atom: u32,
    my_pid: u32,
    px: f64,
    py: f64,
) -> Option<String> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let tree = conn.query_tree(root).ok()?.reply().ok()?;
    for win in tree.children {
        let is_mine = conn
            .get_property(false, win, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut it| it.next()))
            == Some(my_pid);
        if !is_mine {
            continue;
        }
        let name = conn
            .get_property(false, win, name_atom, AtomEnum::ANY, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).into_owned())
            .unwrap_or_default();
        let Some(key) = name.strip_prefix("latte-ball-") else {
            continue;
        };
        let g = conn.get_geometry(win).ok()?.reply().ok()?;
        // 窗口可能被桌面环境强加最小尺寸（xrdp 下 200x200），
        // 可见球是窗口正中央直径 BALL_PHYSICAL_PX 的圆盘，命中判定以圆盘为准
        let (cx, cy) = (g.x as f64 + g.width as f64 / 2.0, g.y as f64 + g.height as f64 / 2.0);
        let r = BALL_PHYSICAL_PX / 2.0 + 4.0; // 4px 容差，好按一点
        if (px - cx).hypot(py - cy) <= r {
            return Some(key.to_string());
        }
    }
    None
}

/// 弹窗头部操作区命中测试：返回 "close"（右上）或 "main"（其次），未命中返回 None。
/// 弹窗内部是 webview，xrdp 下按钮点不动，这两个头部操作由壳原生接管。
/// 区域按 PopupApp.vue 头部布局：高 44px，右侧两个约 44px 宽的按钮位。
#[cfg(target_os = "linux")]
fn hit_popup_chrome(
    conn: &impl x11rb::connection::Connection,
    root: u32,
    pid_atom: u32,
    name_atom: u32,
    my_pid: u32,
    px: f64,
    py: f64,
) -> Option<&'static str> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, MapState};

    let tree = conn.query_tree(root).ok()?.reply().ok()?;
    for win in tree.children {
        let is_mine = conn
            .get_property(false, win, pid_atom, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut it| it.next()))
            == Some(my_pid);
        if !is_mine {
            continue;
        }
        let name = conn
            .get_property(false, win, name_atom, AtomEnum::ANY, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).into_owned())
            .unwrap_or_default();
        if name != "latte-popup" {
            continue;
        }
        // 隐藏中的弹窗不参与命中
        let mapped = conn
            .get_window_attributes(win)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|a| a.map_state == MapState::VIEWABLE);
        if !mapped {
            return None;
        }
        let g = conn.get_geometry(win).ok()?.reply().ok()?;
        let (rx, ry) = (px - g.x as f64, py - g.y as f64);
        let w = g.width as f64;
        const HEADER_H: f64 = 44.0;
        const BTN_W: f64 = 44.0;
        if (0.0..HEADER_H).contains(&ry) && (0.0..w).contains(&rx) {
            if rx >= w - BTN_W {
                return Some("close");
            }
            if rx >= w - BTN_W * 2.0 {
                return Some("main");
            }
        }
        return None;
    }
    None
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

/// 创建 4 个悬浮球窗口和 1 个共享弹窗（需主线程调用，服务就绪后进行）
fn create_floating_windows(app: &tauri::AppHandle) {
    let positions = load_positions();
    let (mon_x, mon_y, mon_w, sf) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| (m.position().x, m.position().y, m.size().width as i32, m.scale_factor()))
        .unwrap_or((0, 0, 1920, 1.0));

    let transparent = ball_transparent();
    #[cfg(target_os = "linux")]
    let native_input = native_ball_input_enabled();
    #[cfg(not(target_os = "linux"))]
    let native_input = false;
    for (i, key) in BALL_KEYS.iter().enumerate() {
        // 默认位置：屏幕右边缘竖排
        let default = (
            mon_x + mon_w - (90.0 * sf) as i32,
            mon_y + (160.0 * sf) as i32 + (i as f64 * 84.0 * sf) as i32,
        );
        let (x, y) = positions.get(*key).copied().unwrap_or(default);
        let mut url = format!("{SERVER_URL}/?ball={key}");
        if !transparent {
            url.push_str("&opaque=1");
        }
        if native_input {
            // 原生输入接管：前端不再处理点击/拖拽
            url.push_str("&native_input=1");
        }
        let win = match WebviewWindowBuilder::new(app, format!("ball-{key}"), WebviewUrl::External(url.parse().unwrap()))
            .title(format!("latte-ball-{key}"))
            .transparent(transparent)
            .decorations(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .shadow(false)
            .focused(false)
            .inner_size(BALL_PHYSICAL_PX, BALL_PHYSICAL_PX)
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

    // 不透明模式（X11 无合成器）：用 X Shape 把球窗裁成正圆球。
    // webview 加载期间窗口会多次重配，shape 可能被重置，
    // 因此延迟多轮重复执行，直至页面稳定后最后一轮生效
    if !transparent {
        tauri::async_runtime::spawn(async {
            for delay_ms in [800u64, 2500, 5000, 8000] {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                apply_round_shape();
            }
        });
    }

    if native_input {
        // X11：悬浮球点击/拖拽由壳原生接管（xrdp 等环境 webview 按键失效）
        start_native_ball_input(app);
    }

    match WebviewWindowBuilder::new(
        app,
        "popup",
        WebviewUrl::External(format!("{SERVER_URL}/?popup=1").parse().unwrap()),
    )
    .title("latte-popup")
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .visible(false)
    .inner_size(POPUP_W, POPUP_H)
    .build()
    {
        Ok(p) => {
            // 失焦自动隐藏（点弹窗以外的地方即收起）
            let p2 = p.clone();
            p.on_window_event(move |event| {
                if let WindowEvent::Focused(false) = event {
                    let _ = p2.hide();
                }
            });
        }
        Err(e) => eprintln!("创建弹窗失败：{e:?}"),
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
