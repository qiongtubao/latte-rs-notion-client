//! Latte — 本地 Notion 客户端（时间碎片 / 金钱 / 日历 / 项目）核心库。
//!
//! 本地优先：SQLite 存储 + dirty 标记，后台同步推送到 Notion；
//! 对外提供 axum HTTP API（见 [api] 模块）。

pub mod ai;
pub mod api;
pub mod config;
pub mod db;
pub mod models;
pub mod notion;
pub mod report;
pub mod sync;
