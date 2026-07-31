//! 配置文件读写：~/.config/latte/config.toml

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub parent_page_id: String,
    #[serde(default)]
    pub events_db_id: String,
    #[serde(default)]
    pub expenses_db_id: String,
    #[serde(default)]
    pub projects_db_id: String,
    /// 知识库根页面 id（setup 时创建；旧配置缺失时由同步任务补建并回填）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_root_page_id: Option<String>,
    /// 「好想法」database id（同步任务懒建并回填；None 表示尚未创建）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ideas_db_id: Option<String>,
    /// 「今日任务」database id（同步任务懒建并回填；None 表示尚未创建）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks_db_id: Option<String>,
    /// 外部 API（/api/ext）Bearer token；配置完成时自动生成，空串表示未生成
    #[serde(default)]
    pub api_token: String,
    #[serde(default)]
    pub ai: AiConfig,
}

/// AI 图片识别配置（OpenAI 兼容代理，全部可选）
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_proxy_base")]
    pub proxy_base: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

fn default_proxy_base() -> String {
    "http://127.0.0.1:16434".to_string()
}

fn default_model() -> String {
    "proxy-default".to_string()
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            proxy_base: default_proxy_base(),
            model: default_model(),
            api_key: String::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("latte")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn db_path() -> PathBuf {
    config_dir().join("latte.db")
}

/// 读取配置；文件不存在时返回 Ok(None)
pub fn load() -> Result<Option<Config>> {
    let path = config_path();
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("无法读取配置文件 {}", path.display()))?;
    let cfg: Config = toml::from_str(&text).context("配置文件格式错误")?;
    Ok(Some(cfg))
}

/// 配置是否完整：token 与 3 个 database id 均非空
pub fn is_configured(cfg: &Config) -> bool {
    !cfg.token.is_empty()
        && !cfg.events_db_id.is_empty()
        && !cfg.expenses_db_id.is_empty()
        && !cfg.projects_db_id.is_empty()
}

pub fn save(cfg: &Config) -> Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(&path, text).with_context(|| format!("无法写入配置文件 {}", path.display()))?;
    Ok(())
}

/// 从 Notion 页面 URL（或纯 id）解析出 32 位十六进制 page id，返回带连字符的标准形式。
///
/// 支持形如：
/// - `https://www.notion.so/My-Page-0123456789abcdef0123456789abcdef?pvs=4`
/// - `01234567-89ab-cdef-0123-456789abcdef`
/// - `0123456789abcdef0123456789abcdef`
pub fn parse_page_id(input: &str) -> Option<String> {
    let s = input.trim().trim_end_matches('/');
    let s = s.split(['?', '#']).next().unwrap_or(s);
    // 从末尾向前收集十六进制字符（跳过连字符），取最后 32 位
    let mut hex: Vec<char> = Vec::with_capacity(32);
    for c in s.chars().rev() {
        if c.is_ascii_hexdigit() {
            hex.push(c.to_ascii_lowercase());
        } else if c == '-' {
            continue;
        } else {
            break;
        }
    }
    if hex.len() < 32 {
        return None;
    }
    hex.truncate(32);
    hex.reverse();
    let raw: String = hex.into_iter().collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &str = "0123456789abcdef0123456789abcdef";
    const HYPHEN: &str = "01234567-89ab-cdef-0123-456789abcdef";

    #[test]
    fn config_roundtrip_with_notes_root_page_id() {
        let mut cfg = Config::default();
        // None 时不落盘，读取回来仍是 None
        let text = toml::to_string_pretty(&cfg).unwrap();
        assert!(!text.contains("notes_root_page_id"));
        assert!(!text.contains("ideas_db_id"));
        assert!(!text.contains("tasks_db_id"));
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.notes_root_page_id, None);
        assert_eq!(parsed.ideas_db_id, None);
        assert_eq!(parsed.tasks_db_id, None);
        // Some 时正常往返
        cfg.notes_root_page_id = Some("page-1".into());
        cfg.ideas_db_id = Some("db-1".into());
        cfg.tasks_db_id = Some("db-2".into());
        let text = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.notes_root_page_id.as_deref(), Some("page-1"));
        assert_eq!(parsed.ideas_db_id.as_deref(), Some("db-1"));
        assert_eq!(parsed.tasks_db_id.as_deref(), Some("db-2"));
    }

    #[test]
    fn parse_full_url() {
        let url = format!("https://www.notion.so/Latte-Data-{RAW}?pvs=4");
        assert_eq!(parse_page_id(&url).as_deref(), Some(HYPHEN));
    }

    #[test]
    fn parse_hyphenated_id() {
        assert_eq!(parse_page_id(HYPHEN).as_deref(), Some(HYPHEN));
    }

    #[test]
    fn parse_raw_id() {
        assert_eq!(parse_page_id(RAW).as_deref(), Some(HYPHEN));
    }

    #[test]
    fn parse_url_with_trailing_slash_and_fragment() {
        let url = format!("https://www.notion.so/{HYPHEN}#anchor");
        assert_eq!(parse_page_id(&url).as_deref(), Some(HYPHEN));
    }

    #[test]
    fn parse_garbage_returns_none() {
        assert_eq!(parse_page_id("hello world"), None);
        assert_eq!(parse_page_id("1234"), None);
        assert_eq!(parse_page_id(""), None);
    }
}
