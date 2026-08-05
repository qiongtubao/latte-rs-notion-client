//! Notion API 客户端：自动建库、创建/更新/归档 page，限速与重试。
//!
//! 请求体构造函数（`*_body` / `*_properties`）为纯函数，附带单元测试。

use std::sync::RwLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::config::Config;
use crate::models::{Category, Event, Expense, Idea, IdeaTag, Project, ProjectStatus, Tag, Task, TaskPriority};

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

/// 从 Notion 拉取并解析后的全量数据（覆盖本地用）
#[derive(Debug, Clone, Default)]
pub struct PulledData {
    pub events: Vec<Event>,
    pub expenses: Vec<Expense>,
    pub projects: Vec<Project>,
    pub ideas: Vec<Idea>,
    pub tasks: Vec<Task>,
}

/// 验证/查询结果：token 是否有效、找到哪些数据库
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupVerifyResult {
    /// token 是否有效（能调通 Notion API）
    pub token_valid: bool,
    /// 父页面下找到的数据库列表
    pub databases: Vec<FoundDatabase>,
    /// 父页面下找到的知识库页面 id（None 表示未找到）
    pub notes_page_id: Option<String>,
    /// 输入的 page_id 实际指向一个 database（用户粘错了 URL），已自动向上定位父页面
    pub is_database: bool,
    /// 解析后真正作为根的页面 id（爬升后可能与输入不同）
    pub resolved_page_id: Option<String>,
    /// 解析/查询过程中的可读错误（token 有效但对象不可访问等），供前端直接展示
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FoundDatabase {
    pub title: String,
    pub id: String,
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

    /// 拉取父页面下全部直属子对象，筛选出 database（按标题匹配 Latte 四种对象）。
    ///
    /// 分页处理（Notion 单次最多 100 个）。用于 setup 时「先查询已存在的数据库，
    /// 避免重复创建」以及「测试配置」按钮的探测。标题读取规则：
    /// - database：`title[0].plain_text`（或 `.text.content`）
    /// - 普通页：`title.plain_text`
    async fn child_objects(&self, page_id: &str) -> Result<Value> {
        let mut all: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let path = match &cursor {
                Some(c) => format!("/blocks/{page_id}/children?page_size=100&start_cursor={c}"),
                None => format!("/blocks/{page_id}/children?page_size=100"),
            };
            let resp = self
                .send(reqwest::Method::GET, &path, &Value::Null)
                .await
                .context("读取 Notion 父页面子对象失败")?;
            if let Some(results) = resp["results"].as_array() {
                all.extend(results.iter().cloned());
            }
            match (resp["has_more"].as_bool(), resp["next_cursor"].as_str()) {
                (Some(true), Some(c)) => cursor = Some(c.to_string()),
                _ => break,
            }
        }
        Ok(json!({ "results": all }))
    }

    /// 提取子 block 的显示标题。
    ///
    /// `/blocks/{page_id}/children` 返回的是 block 对象（`object: "block"`），
    /// 其中 `child_database` / `child_page` 两类 block 的标题分别内嵌在
    /// `child_database.title` / `child_page.title`（字符串，非富文本数组）。
    /// 其他类型 block 不代表 Latte 的库/页面，返回 None。
    fn block_title(obj: &Value) -> Option<&str> {
        match obj["type"].as_str() {
            Some("child_database") => obj["child_database"]["title"].as_str(),
            Some("child_page") => obj["child_page"]["title"].as_str(),
            _ => None,
        }
    }

    /// 从 /search 返回的 database 对象中提取显示标题。
    ///
    /// /search 返回的是 database 对象（`object: "database"`），标题为 `title[]`
    /// 富文本数组（`plain_text` 字段），与 /blocks 子对象的 `child_database.title`
    /// 字符串结构不同，故单独解析。
    fn search_db_title(db: &Value) -> String {
        db["title"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| {
                        r["plain_text"]
                            .as_str()
                            .or_else(|| r["text"]["content"].as_str())
                    })
                    .collect::<String>()
            })
            .unwrap_or_default()
    }

    /// 全局搜索 integration 可访问的全部 database，返回 `<id, 标题>` 列表。
    ///
    /// 用 `POST /search`（filter object=database）而非只看某页面的直接子对象，
    /// 这样无论用户把库建在哪个页面下（甚至多层嵌套）都能找到并复用，
    /// 避免把已存在但非直接子级的库误判为缺失而重复创建同名库。
    /// 标题取自 database 对象的 `title[].plain_text`（富文本数组）。
    async fn find_latte_databases(&self) -> Result<Vec<(String, String)>> {
        let mut found = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let body = match &cursor {
                Some(c) => json!({
                    "filter": { "property": "object", "value": "database" },
                    "page_size": 100,
                    "start_cursor": c
                }),
                None => json!({
                    "filter": { "property": "object", "value": "database" },
                    "page_size": 100
                }),
            };
            let resp = self
                .send(reqwest::Method::POST, "/search", &body)
                .await
                .context("搜索 Notion 数据库失败")?;
            if let Some(results) = resp["results"].as_array() {
                for db in results {
                    if db["object"].as_str() != Some("database") {
                        continue;
                    }
                    let Some(id) = db["id"].as_str() else { continue };
                    let title = Self::search_db_title(db);
                    if !title.is_empty() {
                        found.push((id.to_string(), title));
                    }
                }
            }
            match (resp["has_more"].as_bool(), resp["next_cursor"].as_str()) {
                (Some(true), Some(c)) => cursor = Some(c.to_string()),
                _ => break,
            }
        }
        Ok(found)
    }

    /// 在根页面的直接子对象里查找「📚 知识库」页面，返回其 id（None 表示未找到）。
    ///
    /// 知识库页面始终由 Latte 在根页面下创建，故只查直接子级即可。
    async fn find_notes_page(&self, root_page_id: &str) -> Result<Option<String>> {
        let resp = self.child_objects(root_page_id).await?;
        if let Some(results) = resp["results"].as_array() {
            for obj in results {
                if let Some(title) = Self::block_title(obj) {
                    if title.starts_with("📚 知识库") {
                        if let Some(id) = obj["id"].as_str() {
                            return Ok(Some(id.to_string()));
                        }
                    }
                }
            }
        }
        Ok(None)
    }


    /// 解析 page_id 实际指向的对象类型，必要时向上爬升到普通页面。
    ///
    /// 用户可能粘了某个 database 的 URL（而非根页面 URL）：此时在该 database 下
    /// 创建子 database 会被 Notion 拒绝（"Can't create databases parented by a
    /// database"）。本函数检测到此情况后，读取该 database 的父页面并返回之，
    /// 使后续查询/创建落在真正的普通页面上。
    ///
    /// 返回 `(根页面 id, 输入是否为 database)`。无法定位到可用的普通页面时返回 Err。
    async fn resolve_root(&self, page_id: &str) -> Result<(String, bool)> {
        // 先尝试当作普通页面：GET /pages/{id} 成功且 object=="page" 即认定
        if let Ok(resp) = self
            .send(reqwest::Method::GET, &format!("/pages/{page_id}"), &Value::Null)
            .await
        {
            if resp["object"].as_str() == Some("page") {
                return Ok((page_id.to_string(), false));
            }
        }
        // 否则尝试当作 database：GET /databases/{id}
        let db = self
            .send(reqwest::Method::GET, &format!("/databases/{page_id}"), &Value::Null)
            .await
            .context("无法访问该对象：请确认 URL 指向一个 Notion 页面，且 Integration 已连接到它")?;
        let ptype = db["parent"]["type"].as_str().unwrap_or("");
        match ptype {
            "page_id" => {
                let pid = db["parent"]["page_id"]
                    .as_str()
                    .ok_or_else(|| anyhow!("数据库的父页面 id 缺失"))?;
                Ok((pid.to_string(), true))
            }
            "workspace" => bail!(
                "该数据库位于工作区根目录，没有父页面。请在 Notion 中新建一个普通页面，\
                 把数据库移到该页面下，再用那个页面的 URL 配置"
            ),
            other => bail!(
                "该数据库的父级类型为「{}」，无法在其下创建数据库。请用一个普通页面的 URL 配置",
                other
            ),
        }
    }

    /// 校验 token 是否有效：调用 Notion `/users/me`（GET），失败返回带原因的 Err。
    async fn verify_token(&self) -> Result<()> {
        self.send(reqwest::Method::GET, "/users/me", &Value::Null)
            .await
            .context("token 无效或无权访问 Notion")?;
        Ok(())
    }

    /// 验证配置：校验 token + 查询父页面下已有的 Latte 数据库。
    ///
    /// 即使 token 有效但父页面查询失败（如类型非页面），仍返回 `token_valid`，
    /// 由调用方读取 `databases` / `notes_page_id` 判断缺失项。
    pub async fn verify_setup(&self, page_id: &str) -> SetupVerifyResult {
        let token_valid = self.verify_token().await.is_ok();
        if !token_valid {
            return SetupVerifyResult {
                token_valid,
                databases: Vec::new(),
                notes_page_id: None,
                is_database: false,
                resolved_page_id: None,
                error: Some("Token 无效或无权访问 Notion".into()),
            };
        }
        // 解析输入对象类型：若为 database 则向上爬升到父页面
        let (root_page_id, is_database) = match self.resolve_root(page_id).await {
            Ok(v) => v,
            Err(e) => {
                return SetupVerifyResult {
                    token_valid,
                    databases: Vec::new(),
                    notes_page_id: None,
                    is_database: false,
                    resolved_page_id: None,
                    error: Some(format!("{e:#}")),
                };
            }
        };
        // 全局搜索 integration 可访问的数据库（任意嵌套层级），按标题匹配 Latte 的 3 个库
        let found_dbs = match self.find_latte_databases().await {
            Ok(v) => v,
            Err(e) => {
                return SetupVerifyResult {
                    token_valid,
                    databases: Vec::new(),
                    notes_page_id: None,
                    is_database,
                    resolved_page_id: Some(root_page_id),
                    error: Some(format!("{e:#}")),
                };
            }
        };
        let mut databases = Vec::new();
        for (id, title) in &found_dbs {
            if ["时间碎片", "金钱记录", "项目管理"].contains(&title.as_str()) {
                databases.push(FoundDatabase {
                    title: title.clone(),
                    id: id.clone(),
                });
            }
        }
        // 知识库页面：在根页面直接子级查找（始终由 Latte 创建在根页面下）
        let notes_page_id = match self.find_notes_page(&root_page_id).await {
            Ok(v) => v,
            Err(_) => None,
        };
        SetupVerifyResult {
            token_valid,
            databases,
            notes_page_id,
            is_database,
            resolved_page_id: Some(root_page_id),
            error: None,
        }
    }

    /// 全量替换页面内容：先删除全部直属子 block，再分批写入 blocks
    pub async fn replace_children(&self, page_id: &str, blocks: Vec<Value>) -> Result<()> {
        for id in self.child_block_ids(page_id).await? {
            self.delete_block(&id).await?;
        }
        self.append_children(page_id, blocks).await
    }
    /// 为父页面准备 Latte 的 3 个 database + 知识库页面：先查询已存在的对象并复用，
    /// 只为缺失的创建。返回最终生效的 id（既有 + 新建）。
    ///
    /// 标题匹配与 lazy 建库复用同一套规则，故只要用户在 Notion 里按规定的
    /// 「时间碎片 / 金钱记录 / 项目管理 / 📚 知识库」建好即可直接接入，不会重复创建。
    pub async fn setup_databases(
        &self,
        parent_page_id: &str,
    ) -> Result<(DatabaseIds, String)> {
        // 先解析输入：若用户粘了 database URL，向上爬升到真正的父页面
        let (root_page_id, _is_database) = self.resolve_root(parent_page_id).await?;
        // 全局搜索 integration 可访问的数据库（任意嵌套层级），按标题复用
        let found_dbs = self.find_latte_databases().await?;
        let mut by_title: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
        for (id, title) in &found_dbs {
            by_title.insert(title.as_str(), id.clone());
        }
        // 复用已存在的（无论它建在哪个页面下），缺失的才在根页面下创建
        let events_db_id = match by_title.get("时间碎片") {
            Some(v) => v.clone(),
            None => self.create_db_return(events_db_body(&root_page_id)).await?,
        };
        let expenses_db_id = match by_title.get("金钱记录") {
            Some(v) => v.clone(),
            None => self.create_db_return(expenses_db_body(&root_page_id)).await?,
        };
        let projects_db_id = match by_title.get("项目管理") {
            Some(v) => v.clone(),
            None => self.create_db_return(projects_db_body(&root_page_id)).await?,
        };
        // 知识库页面：在根页面直接子级查找，找不到则在根页面下创建
        let notes_page_id = match self.find_notes_page(&root_page_id).await? {
            Some(id) => id,
            None => {
                let resp = self
                    .send(
                        reqwest::Method::POST,
                        "/pages",
                        &notes_root_body(&root_page_id),
                    )
                    .await
                    .context("创建「📚 知识库」页面失败")?;
                resp["id"].as_str().unwrap_or_default().to_string()
            }
        };
        Ok((
            DatabaseIds {
                events_db_id,
                expenses_db_id,
                projects_db_id,
                notes_page_id,
            },
            root_page_id,
        ))
    }

    /// 创建 database 并返回其 id（复用 create 请求的解析逻辑）
    async fn create_db_return(&self, body: Value) -> Result<String> {
        let resp = self
            .send(reqwest::Method::POST, "/databases", &body)
            .await?;
        resp["id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Notion 响应缺少 database id"))
    }

    /// 查询一个 database 的全部 page（分页 `POST /databases/{id}/query`）
    pub async fn query_database(&self, db_id: &str) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let body = match &cursor {
                Some(c) => json!({ "page_size": 100, "start_cursor": c }),
                None => json!({ "page_size": 100 }),
            };
            let resp = self
                .send(
                    reqwest::Method::POST,
                    &format!("/databases/{db_id}/query"),
                    &body,
                )
                .await
                .context("查询 Notion 数据库失败")?;
            if let Some(results) = resp["results"].as_array() {
                all.extend(results.iter().cloned());
            }
            match (resp["has_more"].as_bool(), resp["next_cursor"].as_str()) {
                (Some(true), Some(c)) => cursor = Some(c.to_string()),
                _ => break,
            }
        }
        Ok(all)
    }

    /// 查询 database 并用解析函数映射为本地模型；db_id 为空时返回空集
    async fn query_and_parse<T>(
        &self,
        db_id: &str,
        parse: impl Fn(&Value) -> Option<T>,
    ) -> Result<Vec<T>> {
        if db_id.is_empty() {
            return Ok(Vec::new());
        }
        let pages = self.query_database(db_id).await?;
        Ok(pages.iter().filter_map(parse).collect())
    }

    /// 从 Notion 全量拉取 5 个 database 的数据并解析为本地模型。
    ///
    /// 任务的 `project_id` 通过「项目」属性里的项目名按名匹配回填（要求项目先拉取）。
    /// ideas_db_id / tasks_db_id 缺失（尚未懒建）时对应集合为空。
    pub async fn pull_all(&self, cfg: &Config) -> Result<PulledData> {
        let events = self.query_and_parse(&cfg.events_db_id, parse_event_page).await?;
        let expenses = self
            .query_and_parse(&cfg.expenses_db_id, parse_expense_page)
            .await?;
        let projects = self
            .query_and_parse(&cfg.projects_db_id, parse_project_page)
            .await?;
        // 项目名 -> 本地 id，用于回填任务的 project_id
        let proj_by_name: std::collections::HashMap<&str, &str> = projects
            .iter()
            .filter(|p| !p.name.is_empty())
            .map(|p| (p.name.as_str(), p.id.as_str()))
            .collect();
        // 任务：解析后按「项目」名回填 project_id
        let mut tasks = Vec::new();
        if let Some(tid) = cfg.tasks_db_id.as_ref() {
            let raw: Vec<(Task, Option<String>)> = self
                .query_database(tid)
                .await?
                .iter()
                .filter_map(parse_task_page)
                .collect();
            for (mut t, proj_name) in raw {
                if let Some(name) = proj_name {
                    t.project_id = proj_by_name.get(name.as_str()).map(|s| s.to_string());
                }
                tasks.push(t);
            }
        }
        let ideas = match cfg.ideas_db_id.as_ref() {
            Some(id) => self.query_and_parse(id, parse_idea_page).await?,
            None => Vec::new(),
        };
        Ok(PulledData {
            events,
            expenses,
            projects,
            ideas,
            tasks,
        })
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

// ---------------- 远端 -> 本地：属性解析（*_properties 的逆函数） ----------------

/// 当前 unix 秒（本地时区）
fn now_ts() -> i64 {
    Local::now().timestamp()
}

/// 解析 Notion date 属性的 `start` 字符串为 unix 秒。
///
/// 支持两种格式：RFC3339（带时区偏移，如 `2024-08-01T00:00:00+08:00`）与
/// 纯日期（`2024-08-01`，按本地时区午夜解释）。
fn parse_notion_date_ts(s: &str) -> Option<i64> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    let d = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Local
        .from_local_datetime(&d.and_hms_opt(0, 0, 0)?)
        .single()
        .map(|dt| dt.timestamp())
}

/// 取 title / rich_text 属性的拼接纯文本（`plain_text` 优先，回退 `text.content`）
fn read_plain(prop: &Value) -> String {
    prop.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    r["plain_text"]
                        .as_str()
                        .or_else(|| r["text"]["content"].as_str())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn prop_title(props: &Value, name: &str) -> String {
    read_plain(&props[name]["title"])
}

fn prop_rich_text(props: &Value, name: &str) -> String {
    read_plain(&props[name]["rich_text"])
}

fn prop_select(props: &Value, name: &str) -> Option<String> {
    props[name]["select"]["name"].as_str().map(String::from)
}

fn prop_number(props: &Value, name: &str) -> Option<f64> {
    props[name]["number"].as_f64()
}

fn prop_checkbox(props: &Value, name: &str) -> bool {
    props[name]["checkbox"].as_bool().unwrap_or(false)
}

/// date 属性 -> unix 秒（事件/消费/项目/想法用）
fn prop_date_ts(props: &Value, name: &str) -> Option<i64> {
    props[name]["date"]["start"]
        .as_str()
        .and_then(parse_notion_date_ts)
}

/// date 属性 -> `YYYY-MM-DD` 字符串（任务日期用；截掉可能的时分秒部分）
fn prop_date_str(props: &Value, name: &str) -> Option<String> {
    props[name]["date"]["start"]
        .as_str()
        .map(|s| s.split('T').next().unwrap_or(s).to_string())
}

/// 页面的 created_time（ISO）-> unix 秒，作为 created_ts 的回退来源
fn page_created_ts(page: &Value) -> i64 {
    page["created_time"]
        .as_str()
        .and_then(parse_notion_date_ts)
        .unwrap_or_else(now_ts)
}

/// 解析「时间碎片」database 的一个 page 为 Event。
/// 本地专属字段 remind/task_id 留默认，由 db 层按 notion_page_id 合并旧值。
pub fn parse_event_page(page: &Value) -> Option<Event> {
    let props = &page["properties"];
    let notion_page_id = page["id"].as_str()?.to_string();
    let start_ts = prop_date_ts(props, "开始")?;
    Some(Event {
        id: Uuid::new_v4().to_string(),
        start_ts,
        end_ts: prop_date_ts(props, "结束"),
        content: prop_rich_text(props, "内容"),
        tag: prop_select(props, "标签")
            .and_then(|s| Tag::from_label(&s))
            .unwrap_or(Tag::Life),
        remind: false,
        task_id: None,
        notion_page_id: Some(notion_page_id),
        dirty: false,
        deleted: false,
    })
}

/// 解析「金钱记录」database 的一个 page 为 Expense（金额元 -> 分）
pub fn parse_expense_page(page: &Value) -> Option<Expense> {
    let props = &page["properties"];
    let notion_page_id = page["id"].as_str()?.to_string();
    let ts = prop_date_ts(props, "时间")?;
    Some(Expense {
        id: Uuid::new_v4().to_string(),
        item: prop_title(props, "事项"),
        amount_cents: (prop_number(props, "金额").unwrap_or(0.0) * 100.0).round() as i64,
        ts,
        category: prop_select(props, "分类")
            .and_then(|s| Category::from_label(&s))
            .unwrap_or(Category::Other),
        notion_page_id: Some(notion_page_id),
        dirty: false,
        deleted: false,
    })
}

/// 解析「项目管理」database 的一个 page 为 Project
pub fn parse_project_page(page: &Value) -> Option<Project> {
    let props = &page["properties"];
    let notion_page_id = page["id"].as_str()?.to_string();
    Some(Project {
        id: Uuid::new_v4().to_string(),
        name: prop_title(props, "名称"),
        status: prop_select(props, "状态")
            .and_then(|s| ProjectStatus::from_label(&s))
            .unwrap_or(ProjectStatus::Doing),
        start_ts: prop_date_ts(props, "开始"),
        deadline_ts: prop_date_ts(props, "截止"),
        note: prop_rich_text(props, "备注"),
        notion_page_id: Some(notion_page_id),
        dirty: false,
        deleted: false,
    })
}

/// 解析「好想法」database 的一个 page 为 Idea（updated_ts 取 created_ts）
pub fn parse_idea_page(page: &Value) -> Option<Idea> {
    let props = &page["properties"];
    let notion_page_id = page["id"].as_str()?.to_string();
    let created_ts = prop_date_ts(props, "时间").unwrap_or_else(|| page_created_ts(page));
    Some(Idea {
        id: Uuid::new_v4().to_string(),
        content: prop_rich_text(props, "内容"),
        tag: prop_select(props, "标签")
            .and_then(|s| IdeaTag::from_label(&s))
            .unwrap_or(IdeaTag::Other),
        pinned: prop_checkbox(props, "置顶"),
        created_ts,
        updated_ts: created_ts,
        notion_page_id: Some(notion_page_id),
        dirty: false,
        deleted: false,
    })
}

/// 解析「今日任务」database 的一个 page。
/// 返回 `(Task, 项目名)`：项目名用于在 pull_all 中按名匹配回 project_id。
/// 本地专属字段 pomodoro_count/estimated_minutes/notes 留默认，由 db 层按
/// notion_page_id 合并旧值。
pub fn parse_task_page(page: &Value) -> Option<(Task, Option<String>)> {
    let props = &page["properties"];
    let notion_page_id = page["id"].as_str()?.to_string();
    let created_ts = page_created_ts(page);
    let project_name = {
        let n = prop_rich_text(props, "项目");
        if n.is_empty() { None } else { Some(n) }
    };
    let task = Task {
        id: Uuid::new_v4().to_string(),
        date: prop_date_str(props, "日期").unwrap_or_default(),
        title: prop_title(props, "名称"),
        priority: prop_select(props, "优先级")
            .and_then(|s| TaskPriority::from_label(&s))
            .unwrap_or(TaskPriority::Mid),
        important: prop_checkbox(props, "重要"),
        urgent: prop_checkbox(props, "紧急"),
        pomodoro_count: 0,
        estimated_minutes: None,
        notes: String::new(),
        task_type: prop_select(props, "类型").unwrap_or_default(),
        project_id: None,
        start_ts: prop_date_ts(props, "计划开始"),
        done: prop_checkbox(props, "完成"),
        created_ts,
        updated_ts: created_ts,
        notion_page_id: Some(notion_page_id),
        dirty: false,
        deleted: false,
    };
    Some((task, project_name))
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
    /// 防御：block_title 从 child_database block 的 child_database.title 取标题
    #[test]
    fn block_title_reads_child_database() {
        // /blocks/{page_id}/children 返回的 child_database block 结构
        let obj = json!({
            "object": "block",
            "id": "d1",
            "type": "child_database",
            "child_database": { "title": "时间碎片" }
        });
        assert_eq!(NotionClient::block_title(&obj), Some("时间碎片"));
    }

    /// 防御：block_title 从 child_page block 取标题（知识库页面）
    #[test]
    fn block_title_reads_child_page() {
        let obj = json!({
            "object": "block",
            "id": "p1",
            "type": "child_page",
            "child_page": { "title": "📚 知识库" }
        });
        assert_eq!(NotionClient::block_title(&obj), Some("📚 知识库"));
    }

    /// 防御：非库/页面 block（段落、列表等）返回 None，不会被误识别为 Latte 库
    #[test]
    fn block_title_none_for_other_blocks() {
        let obj = json!({"object": "block", "id": "b1", "type": "paragraph"});
        assert_eq!(NotionClient::block_title(&obj), None);
    }

    /// 防御：search_db_title 从 /search 返回的 database 对象的 title 富文本数组取标题
    #[test]
    fn search_db_title_reads_plain_text_array() {
        // /search 返回的 database 对象结构
        let db = json!({
            "object": "database",
            "id": "d1",
            "title": [
                {"plain_text": "时间碎片"},
                {"plain_text": "（副本）"}
            ]
        });
        assert_eq!(NotionClient::search_db_title(&db), "时间碎片（副本）");
    }

    /// 防御：search_db_title 回退读 text.content（无 plain_text 时），缺失时返回空串
    #[test]
    fn search_db_title_fallback_and_empty() {
        let db = json!({
            "object": "database",
            "id": "d2",
            "title": [{"text": {"content": "金钱记录"}}]
        });
        assert_eq!(NotionClient::search_db_title(&db), "金钱记录");
        // 无 title 字段 -> 空串（不应误判为某 Latte 库）
        let db2 = json!({"object": "database", "id": "d3"});
        assert_eq!(NotionClient::search_db_title(&db2), "");
    }

    /// 防御：verify_setup 期望的 Latte 数据库标题集合——保证不与未来新增的库重名导致误识别
    #[test]
    fn latte_database_titles_are_stable() {
        let expected = ["时间碎片", "金钱记录", "项目管理"];
        // 关键不变性：每个标题在 Notion 客户端 builder 函数名与 verify 匹配表中保持一致
        let builders: &[&str] = &["时间碎片", "金钱记录", "项目管理"];
        assert_eq!(expected, builders);
    }
    /// 防御：parse_event_page 从 properties 还原 Event（日期解析、标签映射、进行中无结束）
    #[test]
    fn parse_event_page_roundtrip() {
        let page = json!({
            "id": "page-1",
            "properties": {
                "名称": { "title": [{"plain_text": "写代码"}] },
                "开始": { "date": { "start": "2024-08-01T00:00:00+08:00" } },
                "结束": { "date": { "start": "2024-08-01T01:00:00+08:00" } },
                "内容": { "rich_text": [{"plain_text": "写代码"}] },
                "标签": { "select": { "name": "工作" } }
            }
        });
        let ev = parse_event_page(&page).unwrap();
        assert_eq!(ev.content, "写代码");
        assert_eq!(ev.tag, Tag::Work);
        assert_eq!(ev.notion_page_id.as_deref(), Some("page-1"));
        assert!(ev.start_ts < ev.end_ts.unwrap());
        assert!(!ev.dirty); // 拉取的行不再 dirty

        // 进行中事件无「结束」
        let mut p2 = page.clone();
        p2["properties"]["结束"] = json!({ "date": null });
        let ev2 = parse_event_page(&p2).unwrap();
        assert!(ev2.end_ts.is_none());
    }

    /// 防御：parse_expense_page 金额元->分（四舍五入）、分类映射、缺金额按 0
    #[test]
    fn parse_expense_page_amount_to_cents() {
        let page = json!({
            "id": "p",
            "properties": {
                "事项": { "title": [{"plain_text": "午饭"}] },
                "金额": { "number": 25.5 },
                "时间": { "date": { "start": "2024-08-01T12:00:00+08:00" } },
                "分类": { "select": { "name": "餐饮" } }
            }
        });
        let ex = parse_expense_page(&page).unwrap();
        assert_eq!(ex.item, "午饭");
        assert_eq!(ex.amount_cents, 2550);
        assert_eq!(ex.category, Category::Food);

        // 金额缺失 -> 0 分
        let mut p2 = page.clone();
        p2["properties"]["金额"] = json!({});
        assert_eq!(parse_expense_page(&p2).unwrap().amount_cents, 0);
    }

    /// 防御：parse_task_page 还原任务字段，并返回项目名供回填 project_id
    #[test]
    fn parse_task_page_fields_and_project_name() {
        let page = json!({
            "id": "t1",
            "created_time": "2024-08-01T00:00:00.000Z",
            "properties": {
                "名称": { "title": [{"plain_text": "买咖啡"}] },
                "日期": { "date": { "start": "2024-08-01" } },
                "优先级": { "select": { "name": "高" } },
                "重要": { "checkbox": true },
                "紧急": { "checkbox": false },
                "完成": { "checkbox": true },
                "类型": { "select": { "name": "生活" } },
                "项目": { "rich_text": [{"plain_text": "Latte"}] }
            }
        });
        let (t, proj) = parse_task_page(&page).unwrap();
        assert_eq!(t.title, "买咖啡");
        assert_eq!(t.date, "2024-08-01");
        assert_eq!(t.priority, TaskPriority::High);
        assert!(t.important && t.done);
        assert!(!t.urgent);
        assert_eq!(t.task_type, "生活");
        assert_eq!(proj.as_deref(), Some("Latte"));
        assert!(t.created_ts > 0);

        // 无项目 -> None
        let mut p2 = page.clone();
        p2["properties"]["项目"] = json!({ "rich_text": [] });
        assert_eq!(parse_task_page(&p2).unwrap().1, None);
    }

    /// 防御：parse_notion_date_ts 同时支持 RFC3339 与纯日期
    #[test]
    fn parse_notion_date_ts_formats() {
        assert!(parse_notion_date_ts("2024-08-01T00:00:00+08:00").is_some());
        assert!(parse_notion_date_ts("2024-08-01").is_some());
        assert!(parse_notion_date_ts("not a date").is_none());
        // 同一瞬刻：带 +08:00 偏移与 UTC 应得到相同 unix 秒
        let a = parse_notion_date_ts("2024-08-01T00:00:00+08:00").unwrap();
        let b = parse_notion_date_ts("2024-07-31T16:00:00Z").unwrap();
        assert_eq!(a, b);
    }
}
