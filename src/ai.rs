//! AI 图片识别事件：通过 OpenAI 兼容代理做视觉识别，从图片中抽取日程事件。
//!
//! prompt 构造与响应解析为纯函数（附单元测试）；网络调用集中在 [recognize_events]。

use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveTime, TimeZone};
use serde_json::{Value, json};

use crate::config::AiConfig;
use crate::models::Tag;

/// 代理请求超时
const TIMEOUT: Duration = Duration::from_secs(10);

/// 识别出的一条候选事件
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub start_ts: i64,
    pub end_ts: i64,
    pub content: String,
    pub tag: Tag,
}

/// 识别失败原因
#[derive(Debug)]
pub enum AiError {
    /// 代理连接失败 / 超时 / HTTP 错误
    Unavailable(String),
    /// 模型返回无法解析
    BadFormat,
}

impl AiError {
    pub fn message(&self) -> String {
        match self {
            AiError::Unavailable(detail) => format!("AI 服务不可用: {detail}"),
            AiError::BadFormat => "AI 返回格式异常".to_string(),
        }
    }
}

/// 识别 prompt：要求模型严格只返回 JSON 数组
pub fn build_prompt(date: &str) -> String {
    format!(
        "请识别图片中的日程/课程/安排，抽取其中的事件列表。\
         严格只返回一个 JSON 数组，不要输出任何其他文字或解释。\
         数组每项的格式为：\
         {{\"start\":\"HH:mm\",\"end\":\"HH:mm\",\"content\":\"事件内容\",\"tag\":\"标签\"}}，\
         其中 tag 必须是「工作」「运动」「生活」「学习」「看书」中最接近的一个。\
         没有时间信息的事件不要包含。如果无法识别出任何事件，返回 []。\
         这些事件都发生在 {date} 这一天。"
    )
}

/// 构造标准 OpenAI 视觉请求体（chat/completions）
pub fn build_request_body(
    cfg: &AiConfig,
    image_base64: &str,
    media_type: &str,
    date: &str,
) -> Value {
    json!({
        "model": cfg.model,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": build_prompt(date) },
                {
                    "type": "image_url",
                    "image_url": { "url": format!("data:{media_type};base64,{image_base64}") },
                },
            ],
        }],
    })
}

/// 剥掉 ```json 代码围栏与前后空白
fn strip_fence(text: &str) -> &str {
    let t = text.trim();
    let t = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))
        .unwrap_or(t);
    let t = t.strip_suffix("```").unwrap_or(t);
    t.trim()
}

fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// date 当天某时刻的本地 unix 秒（本地时区不存在该时刻时返回 None，如 DST 跳变）
fn local_ts(date: NaiveDate, t: NaiveTime) -> Option<i64> {
    Local
        .from_local_datetime(&date.and_time(t))
        .single()
        .map(|d| d.timestamp())
}

fn parse_item(item: &Value, date: NaiveDate) -> Option<Suggestion> {
    let start = parse_hhmm(item["start"].as_str()?)?;
    let end = parse_hhmm(item["end"].as_str()?)?;
    let content = item["content"].as_str()?.trim();
    if content.is_empty() {
        return None;
    }
    // 标签非法时默认「生活」
    let tag = item["tag"]
        .as_str()
        .and_then(Tag::from_label)
        .unwrap_or(Tag::Life);
    let start_ts = local_ts(date, start)?;
    let end_ts = local_ts(date, end)?;
    if end_ts <= start_ts {
        return None;
    }
    Some(Suggestion {
        start_ts,
        end_ts,
        content: content.to_string(),
        tag,
    })
}

/// 解析模型回复文本为候选事件列表；整体无法解析为 JSON 数组时返回 None
pub fn parse_suggestions(text: &str, date: NaiveDate) -> Option<Vec<Suggestion>> {
    let value: Value = serde_json::from_str(strip_fence(text)).ok()?;
    let items = value.as_array()?;
    Some(items.iter().filter_map(|i| parse_item(i, date)).collect())
}

/// 调用 OpenAI 兼容代理，识别图片中的事件
pub async fn recognize_events(
    cfg: &AiConfig,
    image_base64: &str,
    media_type: &str,
    date: NaiveDate,
) -> Result<Vec<Suggestion>, AiError> {
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| AiError::Unavailable(e.to_string()))?;
    let url = format!(
        "{}/v1/chat/completions",
        cfg.proxy_base.trim_end_matches('/')
    );
    let body = build_request_body(cfg, image_base64, media_type, &date.to_string());
    let mut req = http.post(&url).json(&body);
    if !cfg.api_key.is_empty() {
        req = req.bearer_auth(&cfg.api_key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AiError::Unavailable(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AiError::Unavailable(format!("HTTP {status}")));
    }
    let value: Value = resp
        .json()
        .await
        .map_err(|e| AiError::Unavailable(e.to_string()))?;
    let content = value["choices"][0]["message"]["content"]
        .as_str()
        .ok_or(AiError::BadFormat)?;
    parse_suggestions(content, date).ok_or(AiError::BadFormat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::local_midnight;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 8, 1).unwrap()
    }

    #[test]
    fn prompt_contains_tags_and_date() {
        let p = build_prompt("2024-08-01");
        for tag in ["工作", "运动", "生活", "学习", "看书"] {
            assert!(p.contains(tag), "prompt 应包含标签 {tag}");
        }
        assert!(p.contains("2024-08-01"));
    }

    #[test]
    fn request_body_is_openai_vision_shape() {
        let cfg = AiConfig {
            model: "m1".into(),
            ..AiConfig::default()
        };
        let body = build_request_body(&cfg, "QUJD", "image/png", "2024-08-01");
        assert_eq!(body["model"], "m1");
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

    #[test]
    fn parse_plain_array() {
        let text = r#"[{"start":"09:00","end":"10:30","content":"晨会","tag":"工作"}]"#;
        let out = parse_suggestions(text, date()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "晨会");
        assert_eq!(out[0].tag, Tag::Work);
        let base = local_midnight(date());
        assert_eq!(out[0].start_ts, base + 9 * 3600);
        assert_eq!(out[0].end_ts, base + 10 * 3600 + 30 * 60);
    }

    #[test]
    fn parse_strips_json_fence_and_noise() {
        let text = "  \n```json\n[{\"start\":\"14:00\",\"end\":\"15:00\",\"content\":\"健身\",\"tag\":\"运动\"}]\n```\n ";
        let out = parse_suggestions(text, date()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tag, Tag::Sport);
        // 无 "json" 标注的围栏也要能剥
        let text2 = "```\n[]\n```";
        assert_eq!(parse_suggestions(text2, date()).unwrap().len(), 0);
    }

    #[test]
    fn parse_empty_array() {
        assert_eq!(parse_suggestions("[]", date()).unwrap().len(), 0);
        assert_eq!(
            parse_suggestions(" ```json\n[]\n``` ", date())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn parse_garbage_is_err() {
        assert!(parse_suggestions("无法识别", date()).is_none());
        assert!(parse_suggestions("{\"a\":1}", date()).is_none());
        assert!(parse_suggestions("[1,2", date()).is_none());
    }

    #[test]
    fn invalid_items_filtered_and_bad_tag_defaults_life() {
        let text = r#"[
            {"start":"09:00","end":"10:00","content":"正常","tag":"摸鱼"},
            {"start":"09:00","end":"08:00","content":"倒序","tag":"工作"},
            {"start":"09:00","end":"09:00","content":"相等","tag":"工作"},
            {"start":"9点","end":"10:00","content":"坏时间","tag":"工作"},
            {"start":"09:00","end":"10:00","content":"  ","tag":"工作"},
            {"start":"09:00","content":"缺字段","tag":"工作"},
            {"start":"25:00","end":"26:00","content":"越界","tag":"工作"}
        ]"#;
        let out = parse_suggestions(text, date()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].content, "正常");
        assert_eq!(out[0].tag, Tag::Life); // 非法标签默认「生活」
    }

    #[test]
    fn hhmm_composes_with_date_near_midnight() {
        // 23:00 → 23:59 应合成当天的本地时间戳（跨午夜前的最后时刻）
        let text = r#"[{"start":"23:00","end":"23:59","content":"夜读","tag":"看书"}]"#;
        let out = parse_suggestions(text, date()).unwrap();
        let base = local_midnight(date());
        assert_eq!(out[0].start_ts, base + 23 * 3600);
        assert_eq!(out[0].end_ts, base + 23 * 3600 + 59 * 60);
    }

    #[test]
    fn ai_error_messages() {
        assert_eq!(
            AiError::Unavailable("timeout".into()).message(),
            "AI 服务不可用: timeout"
        );
        assert_eq!(AiError::BadFormat.message(), "AI 返回格式异常");
    }
}
