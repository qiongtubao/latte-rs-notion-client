//! Notion API 客户端：自动建库、创建/更新/归档 page，限速与重试。
//!
//! 请求体构造函数（`*_body` / `*_properties`）为纯函数，附带单元测试。

use std::sync::RwLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Local, TimeZone};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::models::{Category, Event, Expense, Idea, IdeaTag, Project, Tag, Task, TaskPriority};

pub const BASE_URL: &str = "https://api.notion.com/v1";
pub const NOTION_VERSION: &str = "2022-06-28";
/// Notion 限速约 3 req/s，每次请求间隔至少 350ms
const MIN_INTERVAL: Duration = Duration::from_millis(350);
const MAX_RETRIES: usize = 3;

pub struct DatabaseIds {
    pub events_db_id: String,
    pub expenses_db_id: String,
    pub projects_db_id: String,
    pub notes_page_id: String,
}

pub struct NotionClient {
    http: reqwest::Client,
    /// 放在锁里，便于 setup 接口热更新 token 而无需重建客户端
    token: RwLock<String>,
    last_call: Mutex<Instant>,
}

impl NotionClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap_or_default(),
            token: RwLock::new(token.into()),
            last_call: Mutex::new(Instant::now() - MIN_INTERVAL),
        }
    }

    /// 热更新 token（setup 完成后调用，后台同步任务随之生效）
    pub fn set_token(&self, token: impl Into<String>) {
        if let Ok(mut t) = self.token.write() {
            *t = token.into();
        }
    }

    /// 发送请求：限速（间隔 >= 350ms），429 时按 Retry-After 退避重试（最多 MAX_RETRIES 次）
    async fn send(&self, method: reqwest::Method, path: &str, body: &Value) -> Result<Value> {
        let url = format!("{BASE_URL}{path}");
        let token = self.token.read().map(|t| t.clone()).unwrap_or_default();
        let mut last_err: Option<anyhow::Error> = None;
        for _ in 0..MAX_RETRIES {
            {
                // 持锁限速，串行化所有请求
                let mut last = self.last_call.lock().await;
                let elapsed = last.elapsed();
                if elapsed < MIN_INTERVAL {
                    tokio::time::sleep(MIN_INTERVAL - elapsed).await;
                }
                *last = Instant::now();
            }
            let resp = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&token)
                .header("Notion-Version", NOTION_VERSION);
            // GET/DELETE 不带 JSON body
            let resp = if body.is_null() {
                resp.send().await
            } else {
                resp.json(body).send().await
            };
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow!(e).context("网络请求失败"));
                    continue;
                }
            };
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait_secs = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(2);
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                last_err = Some(anyhow!("HTTP 429 限速"));
                continue;
            }
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            if !status.is_success() {
                bail!("Notion API 错误 {status}: {text}");
            }
            return Ok(serde_json::from_str(&text).unwrap_or(Value::Null));
        }
        Err(last_err.unwrap_or_else(|| anyhow!("请求失败，已达最大重试次数")))
    }

    /// 在父页面下创建 3 个 database + 「📚 知识库」子页面，返回其 id
    pub async fn create_databases(&self, parent_page_id: &str) -> Result<DatabaseIds> {
        let events = self
            .send(
                reqwest::Method::POST,
                "/databases",
                &events_db_body(parent_page_id),
            )
            .await
            .context("创建「时间碎片」数据库失败")?;
        let expenses = self
            .send(
                reqwest::Method::POST,
                "/databases",
                &expenses_db_body(parent_page_id),
            )
            .await
            .context("创建「金钱记录」数据库失败")?;
        let projects = self
            .send(
                reqwest::Method::POST,
                "/databases",
                &projects_db_body(parent_page_id),
            )
            .await
            .context("创建「项目管理」数据库失败")?;
        let notes = self
            .send(
                reqwest::Method::POST,
                "/pages",
                &notes_root_body(parent_page_id),
            )
            .await
            .context("创建「📚 知识库」页面失败")?;
        Ok(DatabaseIds {
            events_db_id: events["id"].as_str().unwrap_or_default().to_string(),
            expenses_db_id: expenses["id"].as_str().unwrap_or_default().to_string(),
            projects_db_id: projects["id"].as_str().unwrap_or_default().to_string(),
            notes_page_id: notes["id"].as_str().unwrap_or_default().to_string(),
        })
    }

    /// 在父页面下懒建「好想法」database，返回 database id
    pub async fn create_ideas_db(&self, parent_page_id: &str) -> Result<String> {
        let resp = self
            .send(
                reqwest::Method::POST,
                "/databases",
                &ideas_db_body(parent_page_id),
            )
            .await
            .context("创建「好想法」数据库失败")?;
        resp["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Notion 响应缺少 database id"))
    }

    /// 在父页面下懒建「今日任务」database，返回 database id
    pub async fn create_tasks_db(&self, parent_page_id: &str) -> Result<String> {
        let resp = self
            .send(
                reqwest::Method::POST,
                "/databases",
                &tasks_db_body(parent_page_id),
            )
            .await
            .context("创建「今日任务」数据库失败")?;
        resp["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Notion 响应缺少 database id"))
    }

    /// 在指定 database 中新建 page，返回 page id
    pub async fn create_page(&self, database_id: &str, properties: &Value) -> Result<String> {
        let body = json!({
            "parent": { "type": "database_id", "database_id": database_id },
            "properties": properties,
        });
        let resp = self
            .send(reqwest::Method::POST, "/pages", &body)
            .await
            .context("创建 Notion 页面失败")?;
        resp["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Notion 响应缺少 page id"))
    }

    pub async fn update_page(&self, page_id: &str, properties: &Value) -> Result<()> {
        let body = json!({ "properties": properties });
        self.send(reqwest::Method::PATCH, &format!("/pages/{page_id}"), &body)
            .await
            .context("更新 Notion 页面失败")?;
        Ok(())
    }

    /// 补充 database 属性定义（同名同类型属性幂等，用于给旧 tasks 库补新增列）
    pub async fn update_database(&self, database_id: &str, properties: &Value) -> Result<()> {
        let body = json!({ "properties": properties });
        self.send(reqwest::Method::PATCH, &format!("/databases/{database_id}"), &body)
            .await
            .context("更新 Notion database 结构失败")?;
        Ok(())
    }

    pub async fn archive_page(&self, page_id: &str) -> Result<()> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/pages/{page_id}"),
            &json!({ "archived": true }),
        )
        .await
        .context("归档 Notion 页面失败")?;
        Ok(())
    }

    /// 在父页面下创建普通子页面（知识库目录/文档），返回 page id
    pub async fn create_child_page(&self, parent_page_id: &str, title: &str) -> Result<String> {
        let body = json!({
            "parent": { "type": "page_id", "page_id": parent_page_id },
            "properties": note_title_properties(title),
        });
        let resp = self
            .send(reqwest::Method::POST, "/pages", &body)
            .await
            .context("创建 Notion 子页面失败")?;
        resp["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Notion 响应缺少 page id"))
    }

    /// 页面全部直属子 block id（分页拉取）
    async fn child_block_ids(&self, page_id: &str) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let path = match &cursor {
                Some(c) => format!("/blocks/{page_id}/children?page_size=100&start_cursor={c}"),
                None => format!("/blocks/{page_id}/children?page_size=100"),
            };
            let resp = self
                .send(reqwest::Method::GET, &path, &Value::Null)
                .await
                .context("读取 Notion 页面内容失败")?;
            if let Some(results) = resp["results"].as_array() {
                ids.extend(
                    results
                        .iter()
                        .filter_map(|b| b["id"].as_str().map(String::from)),
                );
            }
            match (resp["has_more"].as_bool(), resp["next_cursor"].as_str()) {
                (Some(true), Some(c)) => cursor = Some(c.to_string()),
                _ => break,
            }
        }
        Ok(ids)
    }

    async fn delete_block(&self, block_id: &str) -> Result<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/blocks/{block_id}"),
            &Value::Null,
        )
        .await
        .context("删除 Notion block 失败")?;
        Ok(())
    }

    /// 追加子 block，按 Notion 单次上限 100 个分批
    pub async fn append_children(&self, page_id: &str, blocks: Vec<Value>) -> Result<()> {
        for chunk in blocks.chunks(100) {
            self.send(
                reqwest::Method::PATCH,
                &format!("/blocks/{page_id}/children"),
                &json!({ "children": chunk }),
            )
            .await
            .context("追加 Notion 页面内容失败")?;
        }
        Ok(())
    }

    /// 全量替换页面内容：先删除全部直属子 block，再分批写入 blocks
    pub async fn replace_children(&self, page_id: &str, blocks: Vec<Value>) -> Result<()> {
        for id in self.child_block_ids(page_id).await? {
            self.delete_block(&id).await?;
        }
        self.append_children(page_id, blocks).await
    }
}

// ---------------- 以下为纯函数：请求体构造 ----------------

/// 时间戳 → RFC3339（本地时区偏移），Notion date 属性可用
fn fmt_rfc3339(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.to_rfc3339())
        .unwrap_or_default()
}

fn title_prop(text: &str) -> Value {
    json!({ "title": [{ "type": "text", "text": { "content": text } }] })
}

fn rich_text_prop(text: &str) -> Value {
    json!({ "rich_text": [{ "type": "text", "text": { "content": text } }] })
}

fn date_prop(ts: i64) -> Value {
    json!({ "date": { "start": fmt_rfc3339(ts) } })
}

fn select_prop(name: &str) -> Value {
    json!({ "select": { "name": name } })
}

fn number_prop(v: f64) -> Value {
    json!({ "number": v })
}

fn checkbox_prop(v: bool) -> Value {
    json!({ "checkbox": v })
}

fn select_options(labels: &[&str]) -> Value {
    let colors = ["blue", "red", "green", "purple", "yellow"];
    json!(
        labels
            .iter()
            .enumerate()
            .map(|(i, l)| json!({ "name": l, "color": colors[i % colors.len()] }))
            .collect::<Vec<_>>()
    )
}

fn db_body(parent_page_id: &str, title: &str, properties: Value) -> Value {
    json!({
        "parent": { "type": "page_id", "page_id": parent_page_id },
        "title": [{ "type": "text", "text": { "content": title } }],
        "properties": properties,
    })
}

/// 「时间碎片」database：名称(title)、开始(date)、结束(date)、内容(rich_text)、标签(select)
pub fn events_db_body(parent_page_id: &str) -> Value {
    let tag_labels: Vec<&str> = Tag::ALL.iter().map(|t| t.label()).collect();
    db_body(
        parent_page_id,
        "时间碎片",
        json!({
            "名称": { "title": {} },
            "开始": { "date": {} },
            "结束": { "date": {} },
            "内容": { "rich_text": {} },
            "标签": { "select": { "options": select_options(&tag_labels) } },
        }),
    )
}

/// 「金钱记录」database：事项(title)、金额(number)、时间(date)、分类(select)
pub fn expenses_db_body(parent_page_id: &str) -> Value {
    let cat_labels: Vec<&str> = Category::ALL.iter().map(|c| c.label()).collect();
    db_body(
        parent_page_id,
        "金钱记录",
        json!({
            "事项": { "title": {} },
            "金额": { "number": { "format": "yuan" } },
            "时间": { "date": {} },
            "分类": { "select": { "options": select_options(&cat_labels) } },
        }),
    )
}

/// 「项目管理」database：名称(title)、状态(select)、开始(date)、截止(date)、备注(rich_text)
pub fn projects_db_body(parent_page_id: &str) -> Value {
    db_body(
        parent_page_id,
        "项目管理",
        json!({
            "名称": { "title": {} },
            "状态": { "select": { "options": select_options(&["暂存", "待办", "进行中", "已完成", "暂停"]) } },
            "开始": { "date": {} },
            "截止": { "date": {} },
            "备注": { "rich_text": {} },
        }),
    )
}

/// 事件 → Notion page properties
pub fn event_properties(ev: &Event) -> Value {
    let title = if ev.content.is_empty() {
        "未命名事件"
    } else {
        ev.content.as_str()
    };
    let mut props = json!({
        "名称": title_prop(title),
        "开始": date_prop(ev.start_ts),
        "内容": rich_text_prop(&ev.content),
        "标签": select_prop(ev.tag.label()),
    });
    if let Some(end) = ev.end_ts {
        props["结束"] = date_prop(end);
    }
    props
}

/// 消费 → Notion page properties（金额单位：元）
pub fn expense_properties(ex: &Expense) -> Value {
    json!({
        "事项": title_prop(&ex.item),
        "金额": number_prop(ex.amount_cents as f64 / 100.0),
        "时间": date_prop(ex.ts),
        "分类": select_prop(ex.category.label()),
    })
}

/// 「好想法」database：名称(title)、内容(rich_text)、标签(select)、置顶(checkbox)、时间(date)
pub fn ideas_db_body(parent_page_id: &str) -> Value {
    let tag_labels: Vec<&str> = IdeaTag::ALL.iter().map(|t| t.label()).collect();
    db_body(
        parent_page_id,
        "好想法",
        json!({
            "名称": { "title": {} },
            "内容": { "rich_text": {} },
            "标签": { "select": { "options": select_options(&tag_labels) } },
            "置顶": { "checkbox": {} },
            "时间": { "date": {} },
        }),
    )
}

/// 想法标题：content 首行前 30 个字符，为空则「想法」
fn idea_title(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        return "想法".to_string();
    }
    first_line.chars().take(30).collect()
}

/// 好想法 → Notion page properties
pub fn idea_properties(idea: &Idea) -> Value {
    json!({
        "名称": title_prop(&idea_title(&idea.content)),
        "内容": rich_text_prop(&idea.content),
        "标签": select_prop(idea.tag.label()),
        "置顶": checkbox_prop(idea.pinned),
        "时间": date_prop(idea.created_ts),
    })
}

/// 「今日任务」database：名称(title)、日期(date)、优先级(select)、重要(checkbox)、紧急(checkbox)、完成(checkbox)
pub fn tasks_db_body(parent_page_id: &str) -> Value {
    let pri_labels: Vec<&str> = TaskPriority::ALL.iter().map(|p| p.label()).collect();
    let mut body = db_body(
        parent_page_id,
        "今日任务",
        json!({
            "名称": { "title": {} },
            "日期": { "date": {} },
            "优先级": { "select": { "options": select_options(&pri_labels) } },
            "重要": { "checkbox": {} },
            "紧急": { "checkbox": {} },
            "完成": { "checkbox": {} },
        }),
    );
    // 合并新增列（类型/项目/计划开始）
    if let (Some(props), Some(extra)) = (
        body["properties"].as_object_mut(),
        tasks_extra_properties_schema().as_object(),
    ) {
        for (k, v) in extra {
            props.insert(k.clone(), v.clone());
        }
    }
    body
}

/// tasks database 的新增列定义：建库与旧库补 schema（PATCH /databases/{id}）共用。
/// 「类型」为 select，写入时 Notion 会自动创建新选项（自由输入）。
pub fn tasks_extra_properties_schema() -> Value {
    json!({
        "类型": { "select": { "options": [] } },
        "项目": { "rich_text": {} },
        "计划开始": { "date": {} },
    })
}

/// 今日任务 → Notion page properties（date 为本地 YYYY-MM-DD，Notion date 可直接接受）。
/// project_name 为关联项目的名字（无关联/找不到时跳过「项目」属性）。
pub fn task_properties(task: &Task, project_name: Option<&str>) -> Value {
    let mut props = serde_json::Map::new();
    props.insert("名称".into(), title_prop(&task.title));
    props.insert("日期".into(), json!({ "date": { "start": task.date } }));
    props.insert("优先级".into(), select_prop(task.priority.label()));
    props.insert("重要".into(), checkbox_prop(task.important));
    props.insert("紧急".into(), checkbox_prop(task.urgent));
    props.insert("完成".into(), checkbox_prop(task.done));
    if !task.task_type.is_empty() {
        props.insert("类型".into(), select_prop(&task.task_type));
    }
    if let Some(name) = project_name.filter(|n| !n.is_empty()) {
        props.insert("项目".into(), rich_text_prop(name));
    }
    if let Some(ts) = task.start_ts {
        props.insert("计划开始".into(), date_prop(ts));
    }
    Value::Object(props)
}

/// 「📚 知识库」根页面（父页面下的普通子页面）
pub fn notes_root_body(parent_page_id: &str) -> Value {
    json!({
        "parent": { "type": "page_id", "page_id": parent_page_id },
        "properties": note_title_properties("📚 知识库"),
    })
}

/// 知识库页面的标题属性（普通子页面的 title 属性名固定为 "title"）
pub fn note_title_properties(title: &str) -> Value {
    json!({ "title": title_prop(title) })
}

// ---------------- Markdown → Notion blocks ----------------

fn rich_text(content: &str) -> Value {
    json!([{ "type": "text", "text": { "content": content } }])
}

fn text_block(kind: &str, content: &str) -> Value {
    json!({ "type": kind, kind: { "rich_text": rich_text(content) } })
}

/// "1. 内容" → 内容（仅「数字 + ". "」前缀）
fn numbered_item(line: &str) -> Option<&str> {
    let (num, rest) = line.split_once(". ")?;
    if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
        Some(rest)
    } else {
        None
    }
}

/// Markdown → Notion blocks。
///
/// 支持 #/##/### 标题、-/* 无序列表、1. 有序列表、``` 代码块（带语言）、
/// 引用（>）、--- 分隔线与普通段落；空行仅作分段。一行一个 block，
/// 不合并富文本。代码块语言原样透传（空语言为 "plain text"）。
pub fn md_to_blocks(md: &str) -> Vec<Value> {
    let mut blocks = Vec::new();
    let mut lines = md.lines();
    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if let Some(lang) = line.strip_prefix("```") {
            // 代码块：收集到结束围栏（未闭合则吃到文末）
            let mut code: Vec<&str> = Vec::new();
            for l in lines.by_ref() {
                if l.trim_end() == "```" {
                    break;
                }
                code.push(l);
            }
            let language = match lang.trim() {
                "" => "plain text",
                l => l,
            };
            blocks.push(json!({
                "type": "code",
                "code": { "rich_text": rich_text(&code.join("\n")), "language": language },
            }));
            continue;
        }
        let block = if let Some(c) = line.strip_prefix("### ") {
            text_block("heading_3", c)
        } else if let Some(c) = line.strip_prefix("## ") {
            text_block("heading_2", c)
        } else if let Some(c) = line.strip_prefix("# ") {
            text_block("heading_1", c)
        } else if let Some(c) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            text_block("bulleted_list_item", c)
        } else if let Some(c) = numbered_item(line) {
            text_block("numbered_list_item", c)
        } else if let Some(c) = line.strip_prefix("> ") {
            text_block("quote", c)
        } else if line == "---" {
            json!({ "type": "divider", "divider": {} })
        } else if line.is_empty() {
            continue;
        } else {
            text_block("paragraph", line)
        };
        blocks.push(block);
    }
    blocks
}

/// 外部记录 → 页面内容 blocks：props 的 json 代码块 + content_md 转换结果。
/// props 为空 / "{}" 时不产生代码块；能解析的 JSON 美化输出。
pub fn ext_record_blocks(props_json: &str, content_md: &str) -> Vec<Value> {
    let mut blocks = Vec::new();
    let trimmed = props_json.trim();
    if !trimmed.is_empty() && trimmed != "{}" {
        let pretty = serde_json::from_str::<Value>(trimmed)
            .and_then(|v| serde_json::to_string_pretty(&v))
            .unwrap_or_else(|_| trimmed.to_string());
        blocks.push(json!({
            "type": "code",
            "code": { "rich_text": rich_text(&pretty), "language": "json" },
        }));
    }
    blocks.extend(md_to_blocks(content_md));
    blocks
}

/// 项目 → Notion page properties
pub fn project_properties(p: &Project) -> Value {
    let mut props = json!({
        "名称": title_prop(&p.name),
        "状态": select_prop(p.status.label()),
        "备注": rich_text_prop(&p.note),
    });
    if let Some(start) = p.start_ts {
        props["开始"] = date_prop(start);
    }
    if let Some(deadline) = p.deadline_ts {
        props["截止"] = date_prop(deadline);
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Category, ProjectStatus, Tag};

    fn sample_event() -> Event {
        Event {
            id: "e1".into(),
            start_ts: 1_722_460_800, // 2024-08-01T00:00:00Z
            end_ts: Some(1_722_464_400),
            content: "写代码".into(),
            tag: Tag::Work,
            remind: false,
            task_id: None,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        }
    }

    #[test]
    fn events_db_body_matches_notion_api() {
        let body = events_db_body("parent-id");
        assert_eq!(body["parent"]["type"], "page_id");
        assert_eq!(body["parent"]["page_id"], "parent-id");
        assert_eq!(body["title"][0]["text"]["content"], "时间碎片");
        let props = &body["properties"];
        assert!(props["名称"]["title"].is_object());
        assert!(props["开始"]["date"].is_object());
        assert!(props["结束"]["date"].is_object());
        assert!(props["内容"]["rich_text"].is_object());
        let options = props["标签"]["select"]["options"].as_array().unwrap();
        let names: Vec<&str> = options
            .iter()
            .map(|o| o["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["工作", "运动", "生活", "学习", "看书"]);
    }

    #[test]
    fn expenses_db_body_has_number_and_select() {
        let body = expenses_db_body("p");
        let props = &body["properties"];
        assert!(props["事项"]["title"].is_object());
        assert_eq!(props["金额"]["number"]["format"], "yuan");
        assert!(props["时间"]["date"].is_object());
        let options = props["分类"]["select"]["options"].as_array().unwrap();
        let names: Vec<&str> = options
            .iter()
            .map(|o| o["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["餐饮", "交通", "购物", "娱乐", "其他"]);
    }

    #[test]
    fn projects_db_body_has_status_options() {
        let body = projects_db_body("p");
        let options = body["properties"]["状态"]["select"]["options"]
            .as_array()
            .unwrap();
        let names: Vec<&str> = options
            .iter()
            .map(|o| o["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["暂存", "待办", "进行中", "已完成", "暂停"]);
    }

    #[test]
    fn event_properties_structure() {
        let props = event_properties(&sample_event());
        assert_eq!(props["名称"]["title"][0]["text"]["content"], "写代码");
        let start = props["开始"]["date"]["start"].as_str().unwrap();
        assert!(start.contains('T') && start.starts_with("2024-08-01"));
        assert!(props["结束"]["date"]["start"].is_string());
        assert_eq!(props["标签"]["select"]["name"], "工作");
        assert_eq!(props["内容"]["rich_text"][0]["text"]["content"], "写代码");
    }

    #[test]
    fn event_properties_ongoing_has_no_end() {
        let mut ev = sample_event();
        ev.end_ts = None;
        ev.content.clear();
        let props = event_properties(&ev);
        assert!(props.get("结束").is_none());
        assert_eq!(props["名称"]["title"][0]["text"]["content"], "未命名事件");
    }

    #[test]
    fn expense_properties_amount_in_yuan() {
        let ex = Expense {
            id: "x".into(),
            item: "午饭".into(),
            amount_cents: 2550,
            ts: 1_722_460_800,
            category: Category::Food,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        };
        let props = expense_properties(&ex);
        assert_eq!(props["事项"]["title"][0]["text"]["content"], "午饭");
        assert_eq!(props["金额"]["number"], json!(25.5));
        assert_eq!(props["分类"]["select"]["name"], "餐饮");
        assert!(props["时间"]["date"]["start"].is_string());
    }

    #[test]
    fn project_properties_optional_dates() {
        let mut p = Project {
            id: "p1".into(),
            name: "Latte".into(),
            status: ProjectStatus::Doing,
            start_ts: Some(1_722_460_800),
            deadline_ts: None,
            note: "本地 Notion 客户端".into(),
            notion_page_id: None,
            dirty: true,
            deleted: false,
        };
        let props = project_properties(&p);
        assert_eq!(props["名称"]["title"][0]["text"]["content"], "Latte");
        assert_eq!(props["状态"]["select"]["name"], "进行中");
        assert!(props["开始"]["date"]["start"].is_string());
        assert!(props.get("截止").is_none());
        assert_eq!(
            props["备注"]["rich_text"][0]["text"]["content"],
            "本地 Notion 客户端"
        );

        p.deadline_ts = Some(1_722_460_800);
        let props = project_properties(&p);
        assert!(props["截止"]["date"]["start"].is_string());
    }

    #[test]
    fn ideas_db_body_matches_notion_api() {
        let body = ideas_db_body("parent-id");
        assert_eq!(body["parent"]["type"], "page_id");
        assert_eq!(body["parent"]["page_id"], "parent-id");
        assert_eq!(body["title"][0]["text"]["content"], "好想法");
        let props = &body["properties"];
        assert!(props["名称"]["title"].is_object());
        assert!(props["内容"]["rich_text"].is_object());
        assert!(props["置顶"]["checkbox"].is_object());
        assert!(props["时间"]["date"].is_object());
        let options = props["标签"]["select"]["options"].as_array().unwrap();
        let names: Vec<&str> = options
            .iter()
            .map(|o| o["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["灵感", "待办", "读书", "问题", "其他"]);
    }

    #[test]
    fn idea_properties_structure() {
        let idea = Idea {
            id: "i1".into(),
            content: "做一个本地优先的 Notion 客户端\n第二行不进标题".into(),
            tag: IdeaTag::Inspiration,
            pinned: true,
            created_ts: 1_722_460_800,
            updated_ts: 1_722_460_800,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        };
        let props = idea_properties(&idea);
        assert_eq!(
            props["名称"]["title"][0]["text"]["content"],
            "做一个本地优先的 Notion 客户端"
        );
        assert_eq!(
            props["内容"]["rich_text"][0]["text"]["content"],
            "做一个本地优先的 Notion 客户端\n第二行不进标题"
        );
        assert_eq!(props["标签"]["select"]["name"], "灵感");
        assert_eq!(props["置顶"]["checkbox"], true);
        assert!(props["时间"]["date"]["start"].is_string());
    }

    #[test]
    fn idea_title_truncates_and_falls_back() {
        // 首行超过 30 字符 → 截断（按字符计，中文也算 1 个）
        let long = "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十多余";
        let mut idea = Idea {
            id: "i".into(),
            content: long.to_string(),
            tag: IdeaTag::Other,
            pinned: false,
            created_ts: 1,
            updated_ts: 1,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        };
        let props = idea_properties(&idea);
        let title = props["名称"]["title"][0]["text"]["content"]
            .as_str()
            .unwrap();
        assert_eq!(title.chars().count(), 30);
        assert_eq!(
            title,
            "一二三四五六七八九十一二三四五六七八九十一二三四五六七八九十"
        );

        // 空内容 / 首行为空白 → 标题回退为「想法」
        idea.content.clear();
        assert_eq!(
            idea_properties(&idea)["名称"]["title"][0]["text"]["content"],
            "想法"
        );
        idea.content = "\n第二行".into();
        assert_eq!(
            idea_properties(&idea)["名称"]["title"][0]["text"]["content"],
            "想法"
        );
    }

    #[test]
    fn tasks_db_body_matches_notion_api() {
        let body = tasks_db_body("parent-id");
        assert_eq!(body["parent"]["type"], "page_id");
        assert_eq!(body["parent"]["page_id"], "parent-id");
        assert_eq!(body["title"][0]["text"]["content"], "今日任务");
        let props = &body["properties"];
        assert!(props["名称"]["title"].is_object());
        assert!(props["日期"]["date"].is_object());
        assert!(props["完成"]["checkbox"].is_object());
        let options = props["优先级"]["select"]["options"].as_array().unwrap();
        let names: Vec<&str> = options
            .iter()
            .map(|o| o["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["高", "中", "低"]);
    }

    #[test]
    fn task_properties_structure() {
        let task = Task {
            id: "t1".into(),
            date: "2024-08-01".into(),
            title: "写周报".into(),
            priority: TaskPriority::High,
            important: true,
            urgent: false,
            pomodoro_count: 0,
            estimated_minutes: None,
            notes: String::new(),
            task_type: String::new(),
            project_id: None,
            start_ts: None,
            done: true,
            created_ts: 1_722_460_800,
            updated_ts: 1_722_460_800,
            notion_page_id: None,
            dirty: true,
            deleted: false,
        };
        let props = task_properties(&task, None);
        assert_eq!(props["名称"]["title"][0]["text"]["content"], "写周报");
        assert_eq!(props["日期"]["date"]["start"], "2024-08-01");
        assert_eq!(props["优先级"]["select"]["name"], "高");
        assert_eq!(props["重要"]["checkbox"], true);
        assert_eq!(props["紧急"]["checkbox"], false);
        assert_eq!(props["完成"]["checkbox"], true);
    }

    #[test]
    fn notes_root_body_is_child_page() {
        let body = notes_root_body("parent-id");
        assert_eq!(body["parent"]["type"], "page_id");
        assert_eq!(body["parent"]["page_id"], "parent-id");
        assert_eq!(
            body["properties"]["title"]["title"][0]["text"]["content"],
            "📚 知识库"
        );
    }

    #[test]
    fn md_to_blocks_headings_and_paragraphs() {
        let blocks = md_to_blocks("# 一级\n## 二级\n### 三级\n\n普通段落\n");
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0]["type"], "heading_1");
        assert_eq!(
            blocks[0]["heading_1"]["rich_text"][0]["text"]["content"],
            "一级"
        );
        assert_eq!(blocks[1]["type"], "heading_2");
        assert_eq!(blocks[2]["type"], "heading_3");
        assert_eq!(blocks[3]["type"], "paragraph");
        assert_eq!(
            blocks[3]["paragraph"]["rich_text"][0]["text"]["content"],
            "普通段落"
        );
        // 空行只分段，不产生 block
        assert!(md_to_blocks("\n\n").is_empty());
    }

    #[test]
    fn md_to_blocks_lists_quote_divider() {
        let blocks = md_to_blocks("- 甲\n* 乙\n1. 丙\n22. 丁\n> 引用\n---\n1.2 不是列表\n");
        assert_eq!(blocks.len(), 7);
        assert_eq!(blocks[0]["type"], "bulleted_list_item");
        assert_eq!(blocks[1]["type"], "bulleted_list_item");
        assert_eq!(blocks[2]["type"], "numbered_list_item");
        assert_eq!(
            blocks[2]["numbered_list_item"]["rich_text"][0]["text"]["content"],
            "丙"
        );
        assert_eq!(
            blocks[3]["numbered_list_item"]["rich_text"][0]["text"]["content"],
            "丁"
        );
        assert_eq!(blocks[4]["type"], "quote");
        assert_eq!(
            blocks[4]["quote"]["rich_text"][0]["text"]["content"],
            "引用"
        );
        assert_eq!(blocks[5]["type"], "divider");
        // "1.2 不是列表" 无 ". " 分隔，按段落处理
        assert_eq!(blocks[6]["type"], "paragraph");
    }

    #[test]
    fn md_to_blocks_code_fence() {
        let blocks = md_to_blocks("```rust\nfn main() {}\nlet x = 1;\n```\n尾部\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "code");
        assert_eq!(blocks[0]["code"]["language"], "rust");
        assert_eq!(
            blocks[0]["code"]["rich_text"][0]["text"]["content"],
            "fn main() {}\nlet x = 1;"
        );
        assert_eq!(blocks[1]["type"], "paragraph");

        // 无语言 → plain text；未闭合 → 吃到文末
        let blocks = md_to_blocks("```\ncode\n");
        assert_eq!(blocks[0]["code"]["language"], "plain text");
        assert_eq!(blocks[0]["code"]["rich_text"][0]["text"]["content"], "code");
    }

    #[test]
    fn ext_record_blocks_props_and_content() {
        // props 美化 + content_md 拼接
        let blocks = ext_record_blocks("{\"a\":1}", "# 标题");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "code");
        assert_eq!(blocks[0]["code"]["language"], "json");
        assert_eq!(
            blocks[0]["code"]["rich_text"][0]["text"]["content"],
            "{\n  \"a\": 1\n}"
        );
        assert_eq!(blocks[1]["type"], "heading_1");

        // 空 props / "{}" 不产生代码块；空内容 → 无 block
        assert!(ext_record_blocks("{}", "").is_empty());
        assert!(ext_record_blocks("", "").is_empty());
        let blocks = ext_record_blocks("{}", "文字");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["type"], "paragraph");

        // 非法 JSON 原样放入代码块
        let blocks = ext_record_blocks("not json", "");
        assert_eq!(
            blocks[0]["code"]["rich_text"][0]["text"]["content"],
            "not json"
        );
    }
}
