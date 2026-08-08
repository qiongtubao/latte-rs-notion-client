//! AI 图片识别事件：通过 OpenAI 兼容代理做视觉识别，从图片中抽取日程事件。
//!
//! prompt 构造与响应解析为纯函数（附单元测试）；网络调用集中在 [recognize_events]。

use std::time::Duration;

use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde_json::{Value, json};

use crate::config::AiConfig;
use crate::models::{Category, IdeaTag, Tag, TaskPriority};

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

// ---------------- 快速录入（一句话解析） ----------------

/// 快速录入的解析结果（已校验，可直接建单）
#[derive(Clone, Debug, PartialEq)]
pub enum QuickEntry {
    Expense {
        item: String,
        amount: f64,
        category: Category,
        ts: i64,
    },
    Event {
        start_ts: i64,
        end_ts: Option<i64>,
        content: String,
        tag: Tag,
        remind: bool,
    },
    Idea {
        content: String,
        tag: IdeaTag,
    },
    Task {
        title: String,
        date: String,
        priority: TaskPriority,
    },
}

/// 快速录入 prompt：分类 + 按类型返回 JSON 对象；now 为「YYYY-MM-DD HH:mm 周X」
pub fn build_quick_prompt(now: &str) -> String {
    format!(
        "现在是 {now}。请把用户的一句话解析为一条记录，严格只返回一个 JSON 对象，不要输出任何其他文字或解释。\
         先判断类型，再按对应格式返回：\n\
         1. 消费：{{\"type\":\"expense\",\"item\":\"事项\",\"amount\":金额数字,\"category\":\"餐饮\",\"time\":\"YYYY-MM-DD HH:mm\"}}；\
         category 必须是「餐饮」「交通」「购物」「娱乐」「其他」之一；time 缺省为现在。\n\
         2. 日程事件：{{\"type\":\"event\",\"content\":\"内容\",\"tag\":\"工作\",\"start\":\"YYYY-MM-DD HH:mm\",\"end\":\"YYYY-MM-DD HH:mm\",\"remind\":false}}；\
         tag 必须是「工作」「运动」「生活」「学习」「看书」之一；只给开始时间时 end 为开始后 1 小时；\
         说了「提醒/提醒我」时 remind 为 true；没有任何时间信息时不要按事件处理。\n\
         3. 想法：{{\"type\":\"idea\",\"content\":\"内容\",\"tag\":\"灵感\"}}；\
         tag 必须是「灵感」「待办」「读书」「问题」「其他」之一。\n\
         4. 任务：{{\"type\":\"task\",\"title\":\"标题\",\"date\":\"YYYY-MM-DD\",\"priority\":\"中\"}}；\
         priority 必须是「高」「中」「低」之一。\n\
         规则：「明天/下午3点」等相对时间按现在换算；金额单位为元；\
         含「记住/灵感/想法」倾向想法，含「要做/待办/提醒我去做」倾向任务，含金额倾向消费；无法判断时按想法处理。"
    )
}

/// 构造快速录入的 OpenAI 文本请求体
pub fn build_quick_request_body(cfg: &AiConfig, text: &str, now: &str) -> Value {
    json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": build_quick_prompt(now) },
            { "role": "user", "content": text },
        ],
    })
}

/// 解析 "YYYY-MM-DD HH:mm" 为本地 unix 秒
fn parse_local_ts(s: &str) -> Option<i64> {
    let dt = NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M").ok()?;
    Local.from_local_datetime(&dt).single().map(|d| d.timestamp())
}

/// 解析模型回复为快速录入结果；无法解析或字段不合法时返回 None
pub fn parse_quick_entry(text: &str, now: chrono::DateTime<Local>) -> Option<QuickEntry> {
    let value: Value = serde_json::from_str(strip_fence(text)).ok()?;
    let now_ts = now.timestamp();
    let today = now.date_naive().to_string();
    match value["type"].as_str()? {
        "expense" => {
            let item = value["item"].as_str()?.trim();
            let amount = value["amount"].as_f64()?;
            if item.is_empty() || amount <= 0.0 {
                return None;
            }
            let ts = value["time"].as_str().and_then(parse_local_ts).unwrap_or(now_ts);
            Some(QuickEntry::Expense {
                item: item.to_string(),
                amount,
                category: value["category"]
                    .as_str()
                    .and_then(Category::from_label)
                    .unwrap_or(Category::Other),
                ts,
            })
        }
        "event" => {
            let content = value["content"].as_str()?.trim();
            let start_ts = parse_local_ts(value["start"].as_str()?)?;
            if content.is_empty() {
                return None;
            }
            let end_ts = value["end"].as_str().and_then(parse_local_ts);
            if end_ts.is_some_and(|e| e <= start_ts) {
                return None;
            }
            Some(QuickEntry::Event {
                start_ts,
                end_ts,
                content: content.to_string(),
                tag: value["tag"].as_str().and_then(Tag::from_label).unwrap_or(Tag::Life),
                remind: value["remind"].as_bool().unwrap_or(false),
            })
        }
        "idea" => {
            let content = value["content"].as_str()?.trim();
            if content.is_empty() {
                return None;
            }
            Some(QuickEntry::Idea {
                content: content.to_string(),
                tag: value["tag"]
                    .as_str()
                    .and_then(IdeaTag::from_label)
                    .unwrap_or(IdeaTag::Other),
            })
        }
        "task" => {
            let title = value["title"].as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            let date = value["date"]
                .as_str()
                .filter(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
                .unwrap_or(&today)
                .to_string();
            Some(QuickEntry::Task {
                title: title.to_string(),
                date,
                priority: value["priority"]
                    .as_str()
                    .and_then(TaskPriority::from_label)
                    .unwrap_or(TaskPriority::Mid),
            })
        }
        _ => None,
    }
}

/// 调用 OpenAI 兼容代理解析一句话，返回校验后的快速录入结果
pub async fn quick_entry(cfg: &AiConfig, text: &str) -> Result<QuickEntry, AiError> {
    let now = Local::now();
    let now_text = now.format("%Y-%m-%d %H:%M 周%u").to_string();
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| AiError::Unavailable(e.to_string()))?;
    let url = format!(
        "{}/v1/chat/completions",
        cfg.proxy_base.trim_end_matches('/')
    );
    let body = build_quick_request_body(cfg, text, &now_text);
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
    parse_quick_entry(content, now).ok_or(AiError::BadFormat)
}

// ---------------- AI 助手（按面板给出建议 + 可采纳的结构化条目） ----------------

/// 助手返回的「可采纳条目」：与 [QuickEntry] 类似但更通用：
/// - note: 知识库条目（title + markdown）
/// - project: 项目（name + 描述/目标）
/// - daily_item: 打卡项（name + kind 打卡/记录 + unit）
/// 其余（expense/event/idea/task）与 [QuickEntry] 同语义，由前端直接调对应创建接口。
#[derive(Clone, Debug, PartialEq)]
pub enum AiAssistItem {
    Task { title: String, date: String, priority: String },
    Event { content: String, tag: String, start: String, end: String, remind: bool },
    Expense { item: String, amount: f64, category: String, time: Option<String> },
    Idea { content: String, tag: String },
    Note { title: String, content_md: String, kind: String },
    /// 「先出大纲」：仅给标题 + 标题数组，正文由下一步生成
    NoteOutline { title: String, headings: Vec<String> },
    Project { name: String, description: String },
    DailyItem { name: String, kind: String, unit: String },
}

/// 助手结果：`summary` 是模型给的文字说明（思路 / 总结 / 计划解释），
/// `items` 是结构化条目供前端一张张采纳。
#[derive(Clone, Debug, PartialEq)]
pub struct AiAssist {
    pub summary: String,
    pub items: Vec<AiAssistItem>,
}

/// 各面板的「该问什么 / 该返回什么」白名单：避免用户/模型超出范围
const ASSIST_PANEL_PROMPTS: &[(&str, &str)] = &[
    (
        "today",
        "用户想规划今日/本周任务。结合用户的【当前任务清单】避免重复，\
         返回 3-7 条 `task` 条目；按可执行、具体、有截止的标题；priority 用「高/中/低」。",
    ),
    (
        "money",
        "用户想整理记账思路或请 AI 给消费建议。优先返回一段 `summary`（思路/建议），\
         如需新增记账条目，返回 1-5 条 `expense`（item 简明、amount 数字、category 必须是「餐饮/交通/购物/娱乐/其他」之一、time 可省）。",
    ),
    (
        "calendar",
        "用户想安排日程。结合【参考日期】和相对时间（如「明天下午3点」）换算，\
         返回 1-5 条 `event` 条目；content 简明、tag 必须是「工作/运动/生活/学习/看书」之一、\
         start/end 格式「YYYY-MM-DD HH:mm」、仅给开始时间时 end 留空字符串。",
    ),
    (
        "projects",
        "用户想规划项目或给项目出大纲。先返回 1 个 `project`（name 简明、description 写目标与关键节点），\
         再返回 3-10 条 `task` 作为下一步行动；task date 留空时回退今天。",
    ),
    (
        "notes",
        "用户想写一篇知识库文档。根据用户描述返回 1 篇 `note`：\
         title 简明、content_md 用 markdown（标题、列表、要点；不要空文档）；kind 必须是「doc」。\
         若用户处于「先出大纲」模式：返回 1 个 `note_outline` 替代 `note`，字段为 `title` + `headings`(3-6 条)，\
         不要写正文。",
    ),
    (
        "ideas",
        "用户想记录灵感 / 待办 / 读书心得 / 问题。返回 1-5 条 `idea`：content 简明、\
         tag 必须是「灵感/待办/读书/问题/其他」之一；同时若有可执行的下一步，附带 1-3 条 `task`。",
    ),
    (
        "daily",
        "用户想规划打卡项 / 寻求坚持建议。优先返回一段 `summary`（坚持思路 / 习惯建议），\
         如需新增打卡项，返回 1-5 条 `daily_item`：name 简明、kind 必须是「打卡/记录」之一、\
         `打卡` 项 unit 留空、`记录` 项 unit 填单位（如 kg / 分钟 / 次）。",
    ),
];

/// 取面板对应的系统提示；未知面板退回「today」的提示
fn assist_panel_hint(panel: &str) -> &'static str {
    for (k, v) in ASSIST_PANEL_PROMPTS {
        if *k == panel {
            return v;
        }
    }
    ASSIST_PANEL_PROMPTS[0].1
}

/// 组装系统 prompt：面板角色 + 严格 JSON 输出 schema
/// - `mode`: 某些面板的特殊模式（仅 notes 面板识别 "outline"）
/// - `length`: 简短/标准/详尽，仅影响字数引导
pub fn build_assist_prompt(panel: &str, now: &str, mode: &str, length: &str) -> String {
    let hint = assist_panel_hint(panel);
    let note_type = if panel == "notes" && mode == "outline" {
        "note_outline"
    } else {
        "note"
    };
    let length_guide = match length {
        "short" => "整体偏简短，重点 3-5 条；",
        "long" => "尽量详尽，给出示例、对比和延伸阅读；",
        _ => "", // normal / 未指定 → 不做引导
    };
    format!(
        "现在是 {now}。面板：{panel}。{length_guide}{hint}\n\
         严格只返回一个 JSON 对象（不要解释、不要围栏）：\n\
         {{\"summary\":\"可选文字说明，可为空字符串\",\"items\":[...]}}\n\
         items 数组中每项按类型取以下字段（缺字段时整条丢弃）：\n\
         - task: {{\"type\":\"task\",\"title\":\"...\",\"date\":\"YYYY-MM-DD 可省\",\"priority\":\"高/中/低 可省，默认中\"}}\n\
         - event: {{\"type\":\"event\",\"content\":\"...\",\"tag\":\"工作/运动/生活/学习/看书 可省，默认生活\",\"start\":\"YYYY-MM-DD HH:mm\",\"end\":\"YYYY-MM-DD HH:mm 可省\",\"remind\":bool 可省}}\n\
         - expense: {{\"type\":\"expense\",\"item\":\"...\",\"amount\":数字,\"category\":\"餐饮/交通/购物/娱乐/其他\",\"time\":\"YYYY-MM-DD HH:mm 可省\"}}\n\
         - idea: {{\"type\":\"idea\",\"content\":\"...\",\"tag\":\"灵感/待办/读书/问题/其他 可省，默认灵感\"}}\n\
         - {note_type}: {{\"type\":\"{note_type}\",\"title\":\"...\",\"content_md\":\"markdown 内容（仅 {note_type} != note_outline 时需要）\",\"headings\":[\"小节标题\"...]（仅 {note_type} == note_outline 时需要，3-6 条）\",\"kind\":\"doc 默认 doc\"}}\n\
         - project: {{\"type\":\"project\",\"name\":\"...\",\"description\":\"目标与关键节点\"}}\n\
         - daily_item: {{\"type\":\"daily_item\",\"name\":\"...\",\"kind\":\"打卡/记录\",\"unit\":\"单位 打卡可省 记录必填\"}}\n\
         当用户没要求新增条目时 items 可以是 []。"
    )
}

pub fn build_assist_request_body(
    cfg: &AiConfig,
    panel: &str,
    text: &str,
    now: &str,
    context: &str,
    mode: &str,
    length: &str,
) -> Value {
    let mut user_msg = text.to_string();
    if !context.is_empty() {
        user_msg.push_str("\n\n【参考上下文】\n");
        user_msg.push_str(context);
    }
    json!({
        "model": cfg.model,
        "messages": [
            { "role": "system", "content": build_assist_prompt(panel, now, mode, length) },
            { "role": "user", "content": user_msg },
        ],
    })
}


/// 把 "高/中/低" / "工作/运动/生活/学习/看书" / 分类 / 想法 tag 等尽量映射成标准标签；
/// 映射失败时回退到默认值（不丢弃整条）
fn norm_priority(s: Option<&str>) -> String {
    match s.unwrap_or("").trim() {
        "高" | "紧急" | "high" | "High" | "HIGH" => "高".into(),
        "低" | "low" | "Low" | "LOW" => "低".into(),
        "中" | "普通" | "mid" | "Med" => "中".into(),
        _ => "中".into(),
    }
}
fn norm_event_tag(s: Option<&str>) -> String {
    match s.unwrap_or("").trim() {
        "工作" | "Work" => "工作".into(),
        "运动" | "Sport" => "运动".into(),
        "学习" | "Study" => "学习".into(),
        "看书" | "Reading" => "看书".into(),
        _ => "生活".into(),
    }
}
fn norm_expense_cat(s: Option<&str>) -> String {
    match s.unwrap_or("").trim() {
        "餐饮" | "吃饭" | "Food" => "餐饮".into(),
        "交通" | "Transport" => "交通".into(),
        "购物" | "Shop" => "购物".into(),
        "娱乐" | "Fun" => "娱乐".into(),
        _ => "其他".into(),
    }
}
fn norm_idea_tag(s: Option<&str>) -> String {
    match s.unwrap_or("").trim() {
        "灵感" | "Inspiration" => "灵感".into(),
        "待办" | "Todo" => "待办".into(),
        "读书" | "Reading" => "读书".into(),
        "问题" | "Question" => "问题".into(),
        _ => "其他".into(),
    }
}
fn norm_daily_kind(s: Option<&str>) -> String {
    match s.unwrap_or("").trim() {
        "打卡" | "习惯" | "habit" => "打卡".into(),
        _ => "记录".into(),
    }
}

/// 解析单个 item：字段缺失/不合法时返回 None
fn parse_assist_item(v: &Value, today: &str) -> Option<AiAssistItem> {
    let t = v["type"].as_str()?;
    match t {
        "task" => {
            let title = v["title"].as_str()?.trim();
            if title.is_empty() { return None; }
            let date = v["date"].as_str()
                .filter(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok())
                .unwrap_or(today)
                .to_string();
            Some(AiAssistItem::Task {
                title: title.into(),
                date,
                priority: norm_priority(v["priority"].as_str()),
            })
        }
        "event" => {
            let content = v["content"].as_str()?.trim();
            if content.is_empty() { return None; }
            let start = v["start"].as_str()?.trim();
            let end = v["end"].as_str().map(|s| s.trim()).filter(|s| !s.is_empty());
            // 校验开始时间合法
            let _ = NaiveDateTime::parse_from_str(start, "%Y-%m-%d %H:%M").ok()?;
            let end_str = end.map(|s| {
                if NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").is_ok() { s.to_string() } else { String::new() }
            }).unwrap_or_default();
            Some(AiAssistItem::Event {
                content: content.into(),
                tag: norm_event_tag(v["tag"].as_str()),
                start: start.into(),
                end: end_str,
                remind: v["remind"].as_bool().unwrap_or(false),
            })
        }
        "expense" => {
            let item = v["item"].as_str()?.trim();
            let amount = v["amount"].as_f64()?;
            if item.is_empty() || amount <= 0.0 { return None; }
            Some(AiAssistItem::Expense {
                item: item.into(),
                amount,
                category: norm_expense_cat(v["category"].as_str()),
                time: v["time"].as_str().map(|s| s.to_string()),
            })
        }
        "idea" => {
            let content = v["content"].as_str()?.trim();
            if content.is_empty() { return None; }
            Some(AiAssistItem::Idea {
                content: content.into(),
                tag: norm_idea_tag(v["tag"].as_str()),
            })
        }
        "note" => {
            let title = v["title"].as_str()?.trim();
            if title.is_empty() { return None; }
            let content_md = v["content_md"].as_str().unwrap_or("").to_string();
            if content_md.trim().is_empty() { return None; }
            let kind = match v["kind"].as_str() {
                Some("dir") => "dir".into(),
                _ => "doc".into(),
            };
            Some(AiAssistItem::Note { title: title.into(), content_md, kind })
        }
        "note_outline" => {
            let title = v["title"].as_str()?.trim();
            if title.is_empty() { return None; }
            let headings: Vec<String> = v["headings"]
                .as_array()
                .map(|arr| arr.iter()
                    .filter_map(|h| h.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .take(8)
                    .collect())
                .unwrap_or_default();
            if headings.is_empty() { return None; }
            Some(AiAssistItem::NoteOutline { title: title.into(), headings })
        }
        "project" => {
            let name = v["name"].as_str()?.trim();
            if name.is_empty() { return None; }
            Some(AiAssistItem::Project {
                name: name.into(),
                description: v["description"].as_str().unwrap_or("").to_string(),
            })
        }
        "daily_item" => {
            let name = v["name"].as_str()?.trim();
            if name.is_empty() { return None; }
            let kind = norm_daily_kind(v["kind"].as_str());
            let unit = v["unit"].as_str().unwrap_or("").trim().to_string();
            // 「记录」必须有 unit，否则归类为「打卡」
            let (kind, unit) = if kind == "记录" && unit.is_empty() {
                ("打卡".to_string(), String::new())
            } else {
                (kind, unit)
            };
            Some(AiAssistItem::DailyItem { name: name.into(), kind, unit })
        }
        _ => None,
    }
}

/// 解析模型回复为 [AiAssist]；JSON 整体无法解析时返回 None
pub fn parse_assist(text: &str, now: chrono::DateTime<Local>) -> Option<AiAssist> {
    let value: Value = serde_json::from_str(strip_fence(text)).ok()?;
    let obj = value.as_object()?;
    let summary = obj["summary"].as_str().unwrap_or("").trim().to_string();
    let items_arr = obj["items"].as_array();
    let today = now.date_naive().to_string();
    let items: Vec<AiAssistItem> = items_arr
        .map(|arr| arr.iter().filter_map(|v| parse_assist_item(v, &today)).collect())
        .unwrap_or_default();
    // summary 和 items 都为空 → 视为无内容，返回 None
    if summary.is_empty() && items.is_empty() {
        return None;
    }
    Some(AiAssist { summary, items })
}

pub async fn ai_assist(
    cfg: &AiConfig,
    panel: &str,
    text: &str,
    context: &str,
    mode: &str,
    length: &str,
) -> Result<AiAssist, AiError> {
    let now = Local::now();
    let now_text = now.format("%Y-%m-%d %H:%M 周%u").to_string();
    let http = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| AiError::Unavailable(e.to_string()))?;
    let url = format!(
        "{}/v1/chat/completions",
        cfg.proxy_base.trim_end_matches('/')
    );
    let body = build_assist_request_body(cfg, panel, text, &now_text, context, mode, length);
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
    parse_assist(content, now).ok_or(AiError::BadFormat)
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

    fn fixed_now() -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2024, 8, 1, 12, 0, 0).single().unwrap()
    }

    #[test]
    fn quick_parse_expense_defaults_time_to_now() {
        let text = r#"{"type":"expense","item":"午饭","amount":25.5,"category":"餐饮","time":null}"#;
        let e = parse_quick_entry(text, fixed_now()).unwrap();
        assert_eq!(
            e,
            QuickEntry::Expense {
                item: "午饭".into(),
                amount: 25.5,
                category: Category::Food,
                ts: fixed_now().timestamp(),
            }
        );
    }

    #[test]
    fn quick_parse_event_with_remind() {
        let text = "```json\n{\"type\":\"event\",\"content\":\"开会\",\"tag\":\"工作\",\"start\":\"2024-08-01 15:00\",\"end\":\"2024-08-01 16:00\",\"remind\":true}\n```";
        match parse_quick_entry(text, fixed_now()).unwrap() {
            QuickEntry::Event {
                start_ts,
                end_ts,
                content,
                tag,
                remind,
            } => {
                assert_eq!(content, "开会");
                assert_eq!(tag, Tag::Work);
                assert!(remind);
                assert_eq!(end_ts.unwrap() - start_ts, 3600);
            }
            other => panic!("应为 Event，实际 {other:?}"),
        }
    }

    #[test]
    fn quick_parse_idea_and_task_fallbacks() {
        let idea = parse_quick_entry(r#"{"type":"idea","content":"做个东西","tag":"不存在"}"#, fixed_now()).unwrap();
        assert_eq!(
            idea,
            QuickEntry::Idea {
                content: "做个东西".into(),
                tag: IdeaTag::Other,
            }
        );
        // 非法日期回退今天、非法优先级回退中
        let task = parse_quick_entry(
            r#"{"type":"task","title":"写周报","date":"bad-date","priority":"急"}"#,
            fixed_now(),
        )
        .unwrap();
        assert_eq!(
            task,
            QuickEntry::Task {
                title: "写周报".into(),
                date: "2024-08-01".into(),
                priority: TaskPriority::Mid,
            }
        );
    }

    #[test]
    fn quick_parse_rejects_bad_input() {
        assert!(parse_quick_entry("无法解析", fixed_now()).is_none());
        assert!(parse_quick_entry(r#"{"type":"x"}"#, fixed_now()).is_none());
        // 空事项 / 非正金额
        assert!(parse_quick_entry(r#"{"type":"expense","item":"","amount":1}"#, fixed_now()).is_none());
        assert!(parse_quick_entry(r#"{"type":"expense","item":"x","amount":-1}"#, fixed_now()).is_none());
        // 事件结束不晚于开始
        assert!(
            parse_quick_entry(
                r#"{"type":"event","content":"x","start":"2024-08-01 16:00","end":"2024-08-01 15:00"}"#,
                fixed_now()
            )
            .is_none()
        );
    }
    // ---- parse_assist ----

    #[test]
    fn assist_parse_mixed_items_and_summary() {
        let text = "{\n            \"summary\": \"本周围绕三件事：... \",\n            \"items\": [\n                {\"type\":\"task\",\"title\":\"写周报\",\"priority\":\"高\",\"date\":\"2024-08-02\"},\n                {\"type\":\"event\",\"content\":\"复盘\",\"tag\":\"工作\",\"start\":\"2024-08-02 16:00\",\"end\":\"2024-08-02 17:00\",\"remind\":true},\n                {\"type\":\"expense\",\"item\":\"咖啡\",\"amount\":18.5,\"category\":\"餐饮\",\"time\":\"2024-08-01 09:00\"},\n                {\"type\":\"idea\",\"content\":\"读《系统之美》\",\"tag\":\"读书\"},\n                {\"type\":\"note\",\"title\":\"Rust 学习路径\",\"content_md\":\"## 入门\\n要点：所有权/借用\",\"kind\":\"doc\"},\n                {\"type\":\"project\",\"name\":\"体重管理\",\"description\":\"三个月减重 5kg\"},\n                {\"type\":\"daily_item\",\"name\":\"早起\",\"kind\":\"打卡\",\"unit\":\"\"},\n                {\"type\":\"daily_item\",\"name\":\"体重\",\"kind\":\"记录\",\"unit\":\"kg\"}\n            ]\n        }";
        let out = parse_assist(text, fixed_now()).unwrap();
        assert_eq!(out.summary, "本周围绕三件事：...");
        assert_eq!(out.items.len(), 8);
        match &out.items[0] {
            AiAssistItem::Task { title, date, priority } => {
                assert_eq!(title, "写周报");
                assert_eq!(date, "2024-08-02");
                assert_eq!(priority, "高");
            }
            other => panic!("应为 Task，实际 {other:?}"),
        }
        match &out.items[1] {
            AiAssistItem::Event { content, tag, start, end, remind } => {
                assert_eq!(content, "复盘");
                assert_eq!(tag, "工作");
                assert_eq!(start, "2024-08-02 16:00");
                assert_eq!(end, "2024-08-02 17:00");
                assert!(remind);
            }
            other => panic!("应为 Event，实际 {other:?}"),
        }
        match &out.items[2] {
            AiAssistItem::Expense { item, amount, category, time } => {
                assert_eq!(item, "咖啡");
                assert!((amount - 18.5).abs() < 1e-9);
                assert_eq!(category, "餐饮");
                assert_eq!(time.as_deref(), Some("2024-08-01 09:00"));
            }
            other => panic!("应为 Expense，实际 {other:?}"),
        }
        match &out.items[3] {
            AiAssistItem::Idea { content, tag } => {
                assert_eq!(content, "读《系统之美》");
                assert_eq!(tag, "读书");
            }
            other => panic!("应为 Idea，实际 {other:?}"),
        }
        match &out.items[4] {
            AiAssistItem::Note { title, content_md, kind } => {
                assert_eq!(title, "Rust 学习路径");
                assert!(content_md.contains("所有权"));
                assert_eq!(kind, "doc");
            }
            other => panic!("应为 Note，实际 {other:?}"),
        }
        match &out.items[5] {
            AiAssistItem::Project { name, description } => {
                assert_eq!(name, "体重管理");
                assert!(description.contains("5kg"));
            }
            other => panic!("应为 Project，实际 {other:?}"),
        }
        // 打卡项 kind 默认值 + unit 校验
        match &out.items[6] {
            AiAssistItem::DailyItem { name, kind, unit } => {
                assert_eq!(name, "早起");
                assert_eq!(kind, "打卡");
                assert_eq!(unit, "");
            }
            other => panic!("应为 DailyItem，实际 {other:?}"),
        }
        match &out.items[7] {
            AiAssistItem::DailyItem { name, kind, unit } => {
                assert_eq!(name, "体重");
                assert_eq!(kind, "记录");
                assert_eq!(unit, "kg");
            }
            other => panic!("应为 DailyItem，实际 {other:?}"),
        }
    }

    #[test]
    fn assist_parse_drops_invalid_items_but_keeps_summary() {
        // 任务空标题 → 丢弃；事件结束时间非法 → 降级为无 end
        let text = r#"{
            "summary": "先这样",
            "items": [
                {"type":"task","title":""},
                {"type":"event","content":"复盘","tag":"工作","start":"2024-08-02 16:00","end":"坏时间","remind":false},
                {"type":"expense","item":"","amount":1},
                {"type":"expense","item":"x","amount":-1}
            ]
        }"#;
        let out = parse_assist(text, fixed_now()).unwrap();
        assert_eq!(out.summary, "先这样");
        // 任务空标题丢弃；事件 end 非法被解析函数降级成空字符串（仍保留）；expense 非法被丢弃
        assert_eq!(out.items.len(), 1);
        match &out.items[0] {
            AiAssistItem::Event { end, .. } => assert_eq!(end, ""),
            other => panic!("应为 Event，实际 {other:?}"),
        }
    }

    #[test]
    fn assist_parse_daily_record_without_unit_falls_back_to_habit() {
        // 「记录」无 unit 应降级为「打卡」
        let text = r#"{"summary":"","items":[{"type":"daily_item","name":"冥想","kind":"记录","unit":""}]}"#;
        let out = parse_assist(text, fixed_now()).unwrap();
        assert_eq!(out.items.len(), 1);
        match &out.items[0] {
            AiAssistItem::DailyItem { kind, unit, .. } => {
                assert_eq!(kind, "打卡");
                assert_eq!(unit, "");
            }
            other => panic!("应为 DailyItem，实际 {other:?}"),
        }
    }

    #[test]
    fn assist_parse_invalid_task_date_falls_back_to_today() {
        let text = r#"{"summary":"","items":[{"type":"task","title":"X","date":"坏","priority":"高"}]}"#;
        let out = parse_assist(text, fixed_now()).unwrap();
        match &out.items[0] {
            AiAssistItem::Task { date, .. } => assert_eq!(date, "2024-08-01"),
            other => panic!("应为 Task，实际 {other:?}"),
        }
    }

    #[test]
    fn assist_parse_empty_returns_none() {
        assert!(parse_assist(r#"{"summary":"","items":[]}"#, fixed_now()).is_none());
        assert!(parse_assist("garbage", fixed_now()).is_none());
    }

    #[test]
    fn assist_panel_hint_covers_all_known_panels() {
        for p in ["today", "money", "calendar", "projects", "notes", "ideas", "daily"] {
            let h = assist_panel_hint(p);
            assert!(!h.is_empty(), "{p} 应有提示");
        }
        // 未知面板退回「today」
        assert!(!assist_panel_hint("unknown").is_empty());
    }

    #[test]
    fn assist_prompt_mentions_panel_and_types() {
        let p = build_assist_prompt("today", "2024-08-01 12:00 周4", "", "");
        assert!(p.contains("today"));
        assert!(p.contains("2024-08-01 12:00 周4"));
        for t in ["task", "event", "expense", "idea", "note", "project", "daily_item"] {
            assert!(p.contains(t), "prompt 应说明 {t} 类型");
        }
        // normal 长度下不出现 short/long 字样
        assert!(!p.contains("偏简短"));
        assert!(!p.contains("尽量详尽"));
    }

    #[test]
    fn assist_prompt_notes_outline_uses_note_outline_type() {
        let p = build_assist_prompt("notes", "2024-08-01 12:00", "outline", "");
        // 标题里出现 note_outline 而不是 note
        assert!(p.contains("note_outline"));
        // 长度引导未指定 → 不出现 short/long 字样
        assert!(!p.contains("偏简短"));
    }

    #[test]
    fn assist_prompt_length_short_and_long_inject_guide() {
        let p_short = build_assist_prompt("today", "2024-08-01 12:00", "", "short");
        assert!(p_short.contains("偏简短"));
        let p_long = build_assist_prompt("today", "2024-08-01 12:00", "", "long");
        assert!(p_long.contains("尽量详尽"));
    }

    #[test]
    fn assist_parse_note_outline_basic() {
        let text = r#"{"summary":"","items":[{"type":"note_outline","title":"Rust 学习路径","headings":["入门","所有权","生命周期","async"]}]}"#;
        let out = parse_assist(text, fixed_now()).unwrap();
        assert_eq!(out.items.len(), 1);
        match &out.items[0] {
            AiAssistItem::NoteOutline { title, headings } => {
                assert_eq!(title, "Rust 学习路径");
                assert_eq!(headings.len(), 4);
                assert_eq!(headings[1], "所有权");
            }
            other => panic!("应为 NoteOutline，实际 {other:?}"),
        }
    }

    #[test]
    fn assist_parse_note_outline_drops_empty_or_oversized() {
        // 空标题 → 丢弃；headings 全空 → 丢弃
        let text = r#"{"summary":"","items":[{"type":"note_outline","title":"","headings":["x"]},{"type":"note_outline","title":"X","headings":[]}]}"#;
        // 两条都被丢弃，且 summary 也空 → parse_assist 返回 None
        assert!(parse_assist(text, fixed_now()).is_none());
        // 只要有非空 summary，即便 items 全空也保留
        let text2 = r#"{"summary":"先想个大纲","items":[{"type":"note_outline","title":"X","headings":[]}]}"#;
        let out = parse_assist(text2, fixed_now()).unwrap();
        assert!(out.items.is_empty());
        assert_eq!(out.summary, "先想个大纲");
    }
}
