//! HTTP API：axum 路由与处理器，统一 JSON 响应。
//!
//! 约定：
//! - 错误统一为 `{"error": "..."}` + 合适状态码
//! - 未完成配置时，除 `/api/status` 与 `/api/setup` 外一律返回 409 `{"error": "not configured"}`
//! - 金额以「元」（浮点）出入 API，本地存储以「分」计
//! - 时间戳均为 unix 秒；日期参数为本地时区的 YYYY-MM-DD

use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::ai;
use crate::config::{self, Config};
use crate::db::Db;
use crate::models::{
    Category, Event, Expense, ExtRecord, Idea, IdeaTag, Note, NoteKind, Project, ProjectStatus,
    Tag, Task, TaskPriority,
};
use crate::notion::NotionClient;
use crate::report::{self, Period};
use crate::sync::{SyncStatus, sync_once};

/// 共享状态
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Db>>,
    pub client: Arc<NotionClient>,
    pub config: Arc<Mutex<Config>>,
    pub sync_status: Arc<Mutex<SyncStatus>>,
}

// ---------------- 错误与工具 ----------------

/// 统一错误响应：`{"error": msg}` + 状态码
struct ApiError(StatusCode, String);

impl ApiError {
    fn new(code: StatusCode, msg: impl Into<String>) -> Self {
        Self(code, msg.into())
    }
    fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, msg)
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, msg)
    }
    fn conflict(msg: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, msg)
    }
    fn internal(err: anyhow::Error) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn now_ts() -> i64 {
    Local::now().timestamp()
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    if s.len() != 10 {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// "YYYY-MM" → 当月 1 号
fn parse_month(s: &str) -> Option<NaiveDate> {
    if s.len() != 7 || s.as_bytes()[4] != b'-' {
        return None;
    }
    NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").ok()
}

fn parse_period(s: &str) -> Option<Period> {
    match s {
        "day" => Some(Period::Day),
        "week" => Some(Period::Week),
        "month" => Some(Period::Month),
        "year" => Some(Period::Year),
        _ => None,
    }
}

/// 区分「字段缺失」与「显式 null」（用于可清空的 Option 字段）
fn de_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(deserializer)?))
}

fn lock_db(state: &AppState) -> ApiResult<MutexGuard<'_, Db>> {
    state
        .db
        .lock()
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "数据库锁不可用"))
}

fn pending_count(state: &AppState) -> usize {
    state
        .db
        .lock()
        .ok()
        .and_then(|g| g.pending_count().ok())
        .unwrap_or(0)
}

fn date_param(q: Option<String>) -> ApiResult<NaiveDate> {
    match q {
        Some(s) => parse_date(&s).ok_or_else(|| ApiError::bad_request("date 格式应为 YYYY-MM-DD")),
        None => Ok(today()),
    }
}

// ---------------- 路由 ----------------

/// 未配置完整时，受保护接口一律 409
async fn require_configured(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let configured = state
        .config
        .lock()
        .map(|c| config::is_configured(&c))
        .unwrap_or(false);
    if !configured {
        return ApiError::conflict("not configured").into_response();
    }
    next.run(req).await
}

/// /api/ext 鉴权：api_token 为空（未完成 Notion 配置）→ 409；
/// 否则要求 `Authorization: Bearer <api_token>`，不匹配 → 401
async fn ext_auth(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let token = state
        .config
        .lock()
        .map(|c| c.api_token.clone())
        .unwrap_or_default();
    if token.is_empty() {
        return ApiError::conflict("not configured").into_response();
    }
    let ok = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == token);
    if !ok {
        return ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }
    next.run(req).await
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/events", get(list_events).post(add_event))
        .route("/api/events/start", post(start_event))
        .route("/api/events/{id}/stop", post(stop_event))
        .route("/api/events/{id}", put(update_event).delete(delete_event))
        .route(
            "/api/ai/recognize-events",
            // 图片 base64 体积大，单独放宽 body limit 到 15MB
            post(recognize_events).layer(DefaultBodyLimit::max(15 * 1024 * 1024)),
        )
        .route("/api/reports/time", get(time_report))
        .route("/api/expenses", get(list_expenses).post(add_expense))
        .route("/api/expenses/summary", get(expenses_summary))
        .route(
            "/api/expenses/{id}",
            put(update_expense).delete(delete_expense),
        )
        .route("/api/calendar", get(calendar_month))
        .route("/api/calendar/day", get(calendar_day))
        .route("/api/projects", get(list_projects).post(add_project))
        .route(
            "/api/projects/{id}",
            put(update_project).delete(delete_project),
        )
        .route("/api/notes/tree", get(notes_tree))
        .route("/api/notes", post(add_note))
        .route(
            "/api/notes/{id}",
            get(get_note).put(update_note).delete(delete_note),
        )
        .route("/api/ideas", get(list_ideas).post(add_idea))
        .route("/api/ideas/{id}", put(update_idea).delete(delete_idea))
        .route("/api/tasks/rollover", post(rollover_tasks))
        .route("/api/tasks/{id}/pomodoro", post(pomodoro_task))
        .route("/api/tasks", get(list_tasks).post(add_task))
        .route("/api/tasks/{id}", put(update_task).delete(delete_task))
        .route("/api/events/ongoing", get(ongoing_event))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_configured,
        ));
    let ext = Router::new()
        .route("/api/ext/{ns}/records", get(ext_list_records))
        .route(
            "/api/ext/{ns}/records/{id}",
            put(ext_upsert_record)
                .get(ext_get_record)
                .delete(ext_delete_record),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), ext_auth));
    Router::new()
        .route("/api/status", get(get_status))
        .route("/api/setup", post(setup))
        .merge(protected)
        .merge(ext)
        .with_state(state)
}

// ---------------- 状态与初始化 ----------------

async fn get_status(State(state): State<AppState>) -> Json<Value> {
    let configured = state
        .config
        .lock()
        .map(|c| config::is_configured(&c))
        .unwrap_or(false);
    let pending = pending_count(&state);
    let (last_sync, last_error) = match state.sync_status.lock() {
        Ok(mut s) => {
            s.pending = pending;
            (s.last_sync.clone(), s.last_error.clone())
        }
        Err(_) => (None, None),
    };
    Json(json!({
        "configured": configured,
        "last_sync": last_sync,
        "last_error": last_error,
        "pending": pending,
    }))
}

#[derive(Deserialize)]
struct SetupBody {
    token: String,
    page_url: String,
}

async fn setup(
    State(state): State<AppState>,
    Json(body): Json<SetupBody>,
) -> ApiResult<Json<Value>> {
    let token = body.token.trim();
    if token.is_empty() {
        return Err(ApiError::bad_request("token 不能为空"));
    }
    let page_id = config::parse_page_id(&body.page_url)
        .ok_or_else(|| ApiError::bad_request("无法从 page_url 解析出 Notion page id"))?;
    // 用新 token 临时建一个客户端去创建 3 个数据库
    let ids = NotionClient::new(token)
        .create_databases(&page_id)
        .await
        .map_err(|e| {
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                format!("创建 Notion 数据库失败: {e:#}"),
            )
        })?;
    // 保留已有 AI 配置、外部 API token 与懒建的 database id，避免重新 setup 时被默认值覆盖
    let (ai_cfg, mut api_token, ideas_db_id, tasks_db_id) = state
        .config
        .lock()
        .map(|c| {
            (
                c.ai.clone(),
                c.api_token.clone(),
                c.ideas_db_id.clone(),
                c.tasks_db_id.clone(),
            )
        })
        .unwrap_or_default();
    if api_token.is_empty() {
        api_token = uuid::Uuid::new_v4().to_string();
    }
    let cfg = Config {
        token: token.to_string(),
        parent_page_id: page_id,
        events_db_id: ids.events_db_id,
        expenses_db_id: ids.expenses_db_id,
        projects_db_id: ids.projects_db_id,
        notes_root_page_id: Some(ids.notes_page_id),
        ideas_db_id,
        tasks_db_id,
        api_token,
        ai: ai_cfg,
    };
    config::save(&cfg).map_err(ApiError::internal)?;
    // 热更新共享状态：后台同步循环下一轮即生效
    state.client.set_token(token);
    if let Ok(mut c) = state.config.lock() {
        *c = cfg;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn sync_now(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let outcome = sync_once(&state.db, &state.client, &state.config).await;
    let pending = pending_count(&state);
    if let Ok(mut s) = state.sync_status.lock() {
        s.last_sync = Some(Local::now().to_rfc3339());
        s.last_error = outcome.error.clone();
        s.pending = pending;
    }
    Ok(Json(json!({ "ok": true, "pending": pending })))
}

// ---------------- 事件 ----------------

fn event_json(ev: &Event, now: i64) -> Value {
    // 进行中的事件尚未填写内容与标签，tag 置 null，避免被误认为默认标签
    let tag = if ev.end_ts.is_none() && ev.content.is_empty() {
        Value::Null
    } else {
        json!(ev.tag.label())
    };
    json!({
        "id": ev.id,
        "start_ts": ev.start_ts,
        "end_ts": ev.end_ts,
        "content": ev.content,
        "tag": tag,
        "remind": ev.remind,
        "duration_seconds": (ev.end_ts.unwrap_or(now) - ev.start_ts).max(0),
    })
}

#[derive(Deserialize)]
struct AddEventBody {
    start_ts: i64,
    end_ts: Option<i64>,
    content: String,
    tag: String,
    #[serde(default)]
    remind: bool,
}

/// 手动补录/规划事件
async fn add_event(
    State(state): State<AppState>,
    Json(body): Json<AddEventBody>,
) -> ApiResult<Json<Value>> {
    let content = body.content.trim();
    if content.is_empty() {
        return Err(ApiError::bad_request("content 不能为空"));
    }
    let tag = Tag::from_label(&body.tag)
        .ok_or_else(|| ApiError::bad_request("无效标签，应为：工作|运动|生活|学习|看书"))?;
    if let Some(end) = body.end_ts
        && end <= body.start_ts
    {
        return Err(ApiError::bad_request("end_ts 应大于 start_ts"));
    }
    let now = now_ts();
    let db = lock_db(&state)?;
    // end_ts 为空表示计划中/进行中，为避免与计时冲突，不允许已有进行中事件
    if body.end_ts.is_none() && db.ongoing_event().map_err(ApiError::internal)?.is_some() {
        return Err(ApiError::conflict("已有进行中事件"));
    }
    let ev = db
        .add_event_full(body.start_ts, body.end_ts, content, tag, body.remind)
        .map_err(ApiError::internal)?;
    Ok(Json(event_json(&ev, now)))
}

#[derive(Deserialize)]
struct EventsQuery {
    date: Option<String>,
}

async fn list_events(
    State(state): State<AppState>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<Json<Value>> {
    let date = date_param(q.date)?;
    let (from, to) = report::day_range(date);
    let now = now_ts();
    let events = lock_db(&state)?
        .events_between(from, to, now)
        .map_err(ApiError::internal)?;
    Ok(Json(Value::Array(
        events.iter().map(|e| event_json(e, now)).collect(),
    )))
}

/// 开始事件；body 可选，提供 content/tag 时预填（任务联动计时用）
#[derive(Deserialize, Default)]
struct StartEventBody {
    content: Option<String>,
    tag: Option<String>,
}

async fn start_event(
    State(state): State<AppState>,
    body: Option<Json<StartEventBody>>,
) -> ApiResult<Json<Value>> {
    let Json(body) = body.unwrap_or_default();
    let tag = body
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| {
            Tag::from_label(t)
                .ok_or_else(|| ApiError::bad_request("无效标签，应为：工作|运动|生活|学习|看书"))
        })
        .transpose()?;
    let content = body.content.as_deref().map(str::trim);
    let now = now_ts();
    let db = lock_db(&state)?;
    if db.ongoing_event().map_err(ApiError::internal)?.is_some() {
        return Err(ApiError::conflict("已有进行中事件"));
    }
    let ev = db.create_event(now).map_err(ApiError::internal)?;
    if content.is_some() || tag.is_some() {
        db.update_event(&ev.id, content, tag, None, None, None)
            .map_err(ApiError::internal)?;
    }
    let ev = db
        .get_event(&ev.id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("事件不存在"))?;
    Ok(Json(event_json(&ev, now)))
}

/// 结束事件；字段均可选：未提供的保留事件现值，
/// 最终 content 为空 → 400，tag 缺省/为空 → 默认「生活」
#[derive(Deserialize, Default)]
struct StopEventBody {
    content: Option<String>,
    tag: Option<String>,
}

async fn stop_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<Json<StopEventBody>>,
) -> ApiResult<Json<Value>> {
    let Json(body) = body.unwrap_or_default();
    let now = now_ts();
    let db = lock_db(&state)?;
    let ev = db
        .get_event(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("事件不存在"))?;
    if ev.end_ts.is_some() {
        return Err(ApiError::conflict("事件已结束"));
    }
    let content = match &body.content {
        Some(c) => c.trim(),
        None => ev.content.trim(),
    };
    if content.is_empty() {
        return Err(ApiError::bad_request("content 不能为空"));
    }
    let tag = match body.tag.as_deref().map(str::trim) {
        Some("") => Tag::Life,
        Some(t) => Tag::from_label(t)
            .ok_or_else(|| ApiError::bad_request("无效标签，应为：工作|运动|生活|学习|看书"))?,
        None => ev.tag,
    };
    db.finish_event(&id, now, content, tag)
        .map_err(ApiError::internal)?;
    let ev = db
        .get_event(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("事件不存在"))?;
    Ok(Json(event_json(&ev, now)))
}

#[derive(Deserialize)]
struct UpdateEventBody {
    content: Option<String>,
    tag: Option<String>,
    start_ts: Option<i64>,
    #[serde(default, deserialize_with = "de_nullable")]
    end_ts: Option<Option<i64>>,
    remind: Option<bool>,
}

async fn update_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateEventBody>,
) -> ApiResult<Json<Value>> {
    let tag = body
        .tag
        .as_deref()
        .map(|t| {
            Tag::from_label(t)
                .ok_or_else(|| ApiError::bad_request("无效标签，应为：工作|运动|生活|学习|看书"))
        })
        .transpose()?;
    let now = now_ts();
    let db = lock_db(&state)?;
    let updated = db
        .update_event(
            &id,
            body.content.as_deref(),
            tag,
            body.start_ts,
            body.end_ts,
            body.remind,
        )
        .map_err(ApiError::internal)?;
    if !updated {
        return Err(ApiError::not_found("事件不存在"));
    }
    let ev = db
        .get_event(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("事件不存在"))?;
    Ok(Json(event_json(&ev, now)))
}

async fn delete_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = lock_db(&state)?;
    if db.get_event(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("事件不存在"));
    }
    db.delete_event(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------- AI 图片识别 ----------------

#[derive(Deserialize)]
struct RecognizeEventsBody {
    image_base64: String,
    media_type: String,
    date: String,
}

/// 识别图片中的日程，返回候选事件列表（不落库，由前端确认后再创建）
async fn recognize_events(
    State(state): State<AppState>,
    Json(body): Json<RecognizeEventsBody>,
) -> ApiResult<Json<Value>> {
    const MEDIA_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];
    if !MEDIA_TYPES.contains(&body.media_type.as_str()) {
        return Err(ApiError::bad_request(
            "media_type 应为 image/png|image/jpeg|image/webp",
        ));
    }
    if body.image_base64.is_empty() {
        return Err(ApiError::bad_request("image_base64 不能为空"));
    }
    let date =
        parse_date(&body.date).ok_or_else(|| ApiError::bad_request("date 格式应为 YYYY-MM-DD"))?;
    let ai_cfg = state
        .config
        .lock()
        .map(|c| c.ai.clone())
        .unwrap_or_default();
    let suggestions = ai::recognize_events(&ai_cfg, &body.image_base64, &body.media_type, date)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e.message()))?;
    Ok(Json(json!({
        "suggestions": suggestions
            .iter()
            .map(|s| json!({
                "start_ts": s.start_ts,
                "end_ts": s.end_ts,
                "content": s.content,
                "tag": s.tag.label(),
            }))
            .collect::<Vec<_>>(),
    })))
}

// ---------------- 报表 ----------------

#[derive(Deserialize)]
struct TimeReportQuery {
    period: Option<String>,
    date: Option<String>,
}

async fn time_report(
    State(state): State<AppState>,
    Query(q): Query<TimeReportQuery>,
) -> ApiResult<Json<Value>> {
    let period = match q.period.as_deref() {
        None => Period::Day,
        Some(p) => parse_period(p)
            .ok_or_else(|| ApiError::bad_request("period 应为 day|week|month|year"))?,
    };
    let date = date_param(q.date)?;
    let (from, to) = report::period_range(period, date);
    let now = now_ts();
    let events = lock_db(&state)?
        .events_between(from, to, now)
        .map_err(ApiError::internal)?;
    let (rows, total) = report::summarize_events(&events, from, to, now);
    let by_tag: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "tag": r.tag.label(),
                "seconds": r.seconds,
                "percent": (r.percent * 10.0).round() / 10.0,
            })
        })
        .collect();
    Ok(Json(json!({
        "range_start": from,
        "range_end": to,
        "total_seconds": total,
        "by_tag": by_tag,
    })))
}

// ---------------- 消费 ----------------

fn expense_json(ex: &Expense) -> Value {
    json!({
        "id": ex.id,
        "item": ex.item,
        "amount_cents": ex.amount_cents,
        "ts": ex.ts,
        "category": ex.category.label(),
    })
}

#[derive(Deserialize)]
struct ExpensesQuery {
    from: Option<String>,
    to: Option<String>,
}

/// from/to 均为 YYYY-MM-DD（to 按当天含尾），缺省为当月
fn expense_range(q: &ExpensesQuery) -> ApiResult<(i64, i64)> {
    let bad = || ApiError::bad_request("日期格式应为 YYYY-MM-DD");
    match (&q.from, &q.to) {
        (None, None) => Ok(report::period_range(Period::Month, today())),
        (Some(f), None) => {
            let d = parse_date(f).ok_or_else(bad)?;
            Ok(report::day_range(d))
        }
        (None, Some(t)) => {
            let d = parse_date(t).ok_or_else(bad)?;
            let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap();
            Ok((
                report::local_midnight(first),
                report::local_midnight(d + Duration::days(1)),
            ))
        }
        (Some(f), Some(t)) => {
            let fd = parse_date(f).ok_or_else(bad)?;
            let td = parse_date(t).ok_or_else(bad)?;
            Ok((
                report::local_midnight(fd),
                report::local_midnight(td + Duration::days(1)),
            ))
        }
    }
}

fn parse_category(s: &str) -> ApiResult<Category> {
    Category::from_label(s)
        .ok_or_else(|| ApiError::bad_request("无效分类，应为：餐饮|交通|购物|娱乐|其他"))
}

fn validate_amount(amount: f64) -> ApiResult<i64> {
    if !amount.is_finite() || amount < 0.0 {
        return Err(ApiError::bad_request("amount 应为非负数字（单位：元）"));
    }
    Ok((amount * 100.0).round() as i64)
}

async fn list_expenses(
    State(state): State<AppState>,
    Query(q): Query<ExpensesQuery>,
) -> ApiResult<Json<Value>> {
    let (from, to) = expense_range(&q)?;
    let expenses = lock_db(&state)?
        .expenses_between(from, to)
        .map_err(ApiError::internal)?;
    Ok(Json(Value::Array(
        expenses.iter().map(expense_json).collect(),
    )))
}

#[derive(Deserialize)]
struct AddExpenseBody {
    item: String,
    amount: f64,
    category: String,
    ts: Option<i64>,
}

async fn add_expense(
    State(state): State<AppState>,
    Json(body): Json<AddExpenseBody>,
) -> ApiResult<Json<Value>> {
    let item = body.item.trim();
    if item.is_empty() {
        return Err(ApiError::bad_request("item 不能为空"));
    }
    let cents = validate_amount(body.amount)?;
    let category = parse_category(&body.category)?;
    let ts = body.ts.unwrap_or_else(now_ts);
    let ex = lock_db(&state)?
        .add_expense(item, cents, ts, category)
        .map_err(ApiError::internal)?;
    Ok(Json(expense_json(&ex)))
}

#[derive(Deserialize)]
struct UpdateExpenseBody {
    item: Option<String>,
    amount: Option<f64>,
    category: Option<String>,
    ts: Option<i64>,
}

async fn update_expense(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateExpenseBody>,
) -> ApiResult<Json<Value>> {
    if let Some(item) = &body.item
        && item.trim().is_empty()
    {
        return Err(ApiError::bad_request("item 不能为空"));
    }
    let cents = body.amount.map(validate_amount).transpose()?;
    let category = body.category.as_deref().map(parse_category).transpose()?;
    let db = lock_db(&state)?;
    let updated = db
        .update_expense(
            &id,
            body.item.as_deref().map(str::trim),
            cents,
            body.ts,
            category,
        )
        .map_err(ApiError::internal)?;
    if !updated {
        return Err(ApiError::not_found("消费记录不存在"));
    }
    let ex = db
        .get_expense(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("消费记录不存在"))?;
    Ok(Json(expense_json(&ex)))
}

async fn delete_expense(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = lock_db(&state)?;
    if db.get_expense(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("消费记录不存在"));
    }
    db.delete_expense(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

async fn expenses_summary(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let date = today();
    let db = lock_db(&state)?;
    let sum_yuan = |period: Period| -> ApiResult<f64> {
        let (from, to) = report::period_range(period, date);
        let expenses = db.expenses_between(from, to).map_err(ApiError::internal)?;
        Ok(report::sum_expenses_in_range(&expenses, from, to) as f64 / 100.0)
    };
    Ok(Json(json!({
        "week": sum_yuan(Period::Week)?,
        "month": sum_yuan(Period::Month)?,
        "year": sum_yuan(Period::Year)?,
    })))
}

// ---------------- 日历 ----------------

#[derive(Deserialize)]
struct CalendarMonthQuery {
    month: Option<String>,
}

async fn calendar_month(
    State(state): State<AppState>,
    Query(q): Query<CalendarMonthQuery>,
) -> ApiResult<Json<Value>> {
    let first = match q.month {
        Some(m) => {
            parse_month(&m).ok_or_else(|| ApiError::bad_request("month 格式应为 YYYY-MM"))?
        }
        None => {
            let t = today();
            NaiveDate::from_ymd_opt(t.year(), t.month(), 1).unwrap()
        }
    };
    let (mfrom, mto) = report::period_range(Period::Month, first);
    let now = now_ts();
    let db = lock_db(&state)?;
    let events = db
        .events_between(mfrom, mto, now)
        .map_err(ApiError::internal)?;
    let expenses = db
        .expenses_between(mfrom, mto)
        .map_err(ApiError::internal)?;
    let mut days = Vec::new();
    let mut d = first;
    while report::local_midnight(d) < mto {
        let (df, dt) = report::day_range(d);
        let seconds: i64 = events
            .iter()
            .map(|e| report::event_seconds_in_range(e, df, dt, now))
            .sum();
        let cents: i64 = expenses
            .iter()
            .filter(|e| e.ts >= df && e.ts < dt)
            .map(|e| e.amount_cents)
            .sum();
        if seconds > 0 || cents > 0 {
            days.push(json!({
                "date": d.format("%Y-%m-%d").to_string(),
                "seconds": seconds,
                "expense": cents as f64 / 100.0,
            }));
        }
        d += Duration::days(1);
    }
    Ok(Json(Value::Array(days)))
}

#[derive(Deserialize)]
struct CalendarDayQuery {
    date: Option<String>,
}

async fn calendar_day(
    State(state): State<AppState>,
    Query(q): Query<CalendarDayQuery>,
) -> ApiResult<Json<Value>> {
    let date = date_param(q.date)?;
    let (from, to) = report::day_range(date);
    let now = now_ts();
    let db = lock_db(&state)?;
    let events = db
        .events_between(from, to, now)
        .map_err(ApiError::internal)?;
    let expenses = db.expenses_between(from, to).map_err(ApiError::internal)?;
    Ok(Json(json!({
        "events": events.iter().map(|e| event_json(e, now)).collect::<Vec<_>>(),
        "expenses": expenses.iter().map(expense_json).collect::<Vec<_>>(),
    })))
}

// ---------------- 项目 ----------------

fn project_json(p: &Project) -> Value {
    json!({
        "id": p.id,
        "name": p.name,
        "status": p.status.label(),
        "start_ts": p.start_ts,
        "deadline_ts": p.deadline_ts,
        "note": p.note,
    })
}

fn parse_status(s: &str) -> ApiResult<ProjectStatus> {
    ProjectStatus::from_label(s)
        .ok_or_else(|| ApiError::bad_request("无效状态，应为：暂存|待办|进行中|已完成|暂停"))
}

async fn list_projects(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let projects = lock_db(&state)?
        .all_projects()
        .map_err(ApiError::internal)?;
    Ok(Json(Value::Array(
        projects.iter().map(project_json).collect(),
    )))
}

#[derive(Deserialize)]
struct AddProjectBody {
    name: String,
    status: Option<String>,
    start_ts: Option<i64>,
    deadline_ts: Option<i64>,
    note: Option<String>,
}

async fn add_project(
    State(state): State<AppState>,
    Json(body): Json<AddProjectBody>,
) -> ApiResult<Json<Value>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("name 不能为空"));
    }
    let status = body
        .status
        .as_deref()
        .map(parse_status)
        .transpose()?
        .unwrap_or(ProjectStatus::Todo);
    let note = body.note.unwrap_or_default();
    let p = lock_db(&state)?
        .add_project(name, status, body.start_ts, body.deadline_ts, &note)
        .map_err(ApiError::internal)?;
    Ok(Json(project_json(&p)))
}

#[derive(Deserialize)]
struct UpdateProjectBody {
    name: Option<String>,
    status: Option<String>,
    #[serde(default, deserialize_with = "de_nullable")]
    start_ts: Option<Option<i64>>,
    #[serde(default, deserialize_with = "de_nullable")]
    deadline_ts: Option<Option<i64>>,
    note: Option<String>,
}

async fn update_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProjectBody>,
) -> ApiResult<Json<Value>> {
    if let Some(name) = &body.name
        && name.trim().is_empty()
    {
        return Err(ApiError::bad_request("name 不能为空"));
    }
    let status = body.status.as_deref().map(parse_status).transpose()?;
    let db = lock_db(&state)?;
    let updated = db
        .update_project(
            &id,
            body.name.as_deref().map(str::trim),
            status,
            body.start_ts,
            body.deadline_ts,
            body.note.as_deref(),
        )
        .map_err(ApiError::internal)?;
    if !updated {
        return Err(ApiError::not_found("项目不存在"));
    }
    let p = db
        .get_project(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("项目不存在"))?;
    Ok(Json(project_json(&p)))
}

async fn delete_project(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = lock_db(&state)?;
    if db.get_project(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("项目不存在"));
    }
    db.delete_project(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------- 知识库 ----------------

/// 条目完整对象（不含同步内部字段）
fn note_json(n: &Note) -> Value {
    json!({
        "id": n.id,
        "parent_id": n.parent_id,
        "kind": n.kind.label(),
        "title": n.title,
        "content_md": n.content_md,
        "created_ts": n.created_ts,
        "updated_ts": n.updated_ts,
    })
}

/// 嵌套树的一层；notes 已按 created_ts 排序
fn note_tree_level(notes: &[Note], parent: Option<&str>) -> Vec<Value> {
    notes
        .iter()
        .filter(|n| n.parent_id.as_deref() == parent)
        .map(|n| {
            json!({
                "id": n.id,
                "parent_id": n.parent_id,
                "kind": n.kind.label(),
                "title": n.title,
                "created_ts": n.created_ts,
                "updated_ts": n.updated_ts,
                "children": note_tree_level(notes, Some(&n.id)),
            })
        })
        .collect()
}

async fn notes_tree(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let notes = lock_db(&state)?.all_notes().map_err(ApiError::internal)?;
    Ok(Json(Value::Array(note_tree_level(&notes, None))))
}

async fn get_note(State(state): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let note = lock_db(&state)?
        .get_note(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("知识库条目不存在"))?;
    Ok(Json(note_json(&note)))
}

/// 校验父节点：存在且为目录
fn validate_note_parent(db: &Db, parent_id: &str) -> ApiResult<()> {
    match db.get_note(parent_id).map_err(ApiError::internal)? {
        None => Err(ApiError::bad_request("父目录不存在")),
        Some(p) if p.kind != NoteKind::Dir => Err(ApiError::bad_request("父节点必须是目录")),
        Some(_) => Ok(()),
    }
}

#[derive(Deserialize)]
struct AddNoteBody {
    /// null / 缺省 = 根
    parent_id: Option<String>,
    kind: String,
    title: String,
    content_md: Option<String>,
}

async fn add_note(
    State(state): State<AppState>,
    Json(body): Json<AddNoteBody>,
) -> ApiResult<Json<Value>> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("title 不能为空"));
    }
    let kind = NoteKind::from_label(&body.kind)
        .ok_or_else(|| ApiError::bad_request("kind 应为 dir|doc"))?;
    let now = now_ts();
    let db = lock_db(&state)?;
    if let Some(pid) = body.parent_id.as_deref() {
        validate_note_parent(&db, pid)?;
    }
    let note = db
        .add_note(
            body.parent_id.as_deref(),
            kind,
            title,
            &body.content_md.unwrap_or_default(),
            now,
        )
        .map_err(ApiError::internal)?;
    Ok(Json(note_json(&note)))
}

#[derive(Deserialize)]
struct UpdateNoteBody {
    title: Option<String>,
    content_md: Option<String>,
    /// 显式 null = 移到根
    #[serde(default, deserialize_with = "de_nullable")]
    parent_id: Option<Option<String>>,
}

async fn update_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateNoteBody>,
) -> ApiResult<Json<Value>> {
    if let Some(title) = &body.title
        && title.trim().is_empty()
    {
        return Err(ApiError::bad_request("title 不能为空"));
    }
    let now = now_ts();
    let db = lock_db(&state)?;
    if db.get_note(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("知识库条目不存在"));
    }
    if let Some(Some(pid)) = &body.parent_id {
        validate_note_parent(&db, pid)?;
        if db.note_would_cycle(&id, pid).map_err(ApiError::internal)? {
            return Err(ApiError::bad_request("不能将目录移动到其自身或子孙节点下"));
        }
    }
    let updated = db
        .update_note(
            &id,
            body.title.as_deref().map(str::trim),
            body.content_md.as_deref(),
            body.parent_id.as_ref().map(|p| p.as_deref()),
            now,
        )
        .map_err(ApiError::internal)?;
    if !updated {
        return Err(ApiError::not_found("知识库条目不存在"));
    }
    let note = db
        .get_note(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("知识库条目不存在"))?;
    Ok(Json(note_json(&note)))
}

/// 递归软删：已同步过的条目待远端归档，未同步过的直接物理删
async fn delete_note(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = lock_db(&state)?;
    if db.get_note(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("知识库条目不存在"));
    }
    db.delete_note_recursive(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------- 好想法 ----------------

/// 想法完整对象（不含同步内部字段）
fn idea_json(i: &Idea) -> Value {
    json!({
        "id": i.id,
        "content": i.content,
        "tag": i.tag.label(),
        "pinned": i.pinned,
        "created_ts": i.created_ts,
        "updated_ts": i.updated_ts,
    })
}

fn parse_idea_tag(s: &str) -> ApiResult<IdeaTag> {
    IdeaTag::from_label(s)
        .ok_or_else(|| ApiError::bad_request("无效标签，应为：灵感|待办|读书|问题|其他"))
}

#[derive(Deserialize)]
struct IdeasQuery {
    tag: Option<String>,
}

/// 列表：置顶优先，再按创建时间倒序；?tag=灵感 过滤
async fn list_ideas(
    State(state): State<AppState>,
    Query(q): Query<IdeasQuery>,
) -> ApiResult<Json<Value>> {
    let tag = q.tag.as_deref().map(parse_idea_tag).transpose()?;
    let ideas = lock_db(&state)?
        .all_ideas(tag)
        .map_err(ApiError::internal)?;
    Ok(Json(Value::Array(ideas.iter().map(idea_json).collect())))
}

#[derive(Deserialize)]
struct AddIdeaBody {
    content: String,
    tag: Option<String>,
    pinned: Option<bool>,
}

async fn add_idea(
    State(state): State<AppState>,
    Json(body): Json<AddIdeaBody>,
) -> ApiResult<Json<Value>> {
    let content = body.content.trim();
    if content.is_empty() {
        return Err(ApiError::bad_request("content 不能为空"));
    }
    let tag = body
        .tag
        .as_deref()
        .map(parse_idea_tag)
        .transpose()?
        .unwrap_or(IdeaTag::Inspiration);
    let idea = lock_db(&state)?
        .add_idea(content, tag, body.pinned.unwrap_or(false), now_ts())
        .map_err(ApiError::internal)?;
    Ok(Json(idea_json(&idea)))
}

#[derive(Deserialize)]
struct UpdateIdeaBody {
    content: Option<String>,
    tag: Option<String>,
    pinned: Option<bool>,
}

async fn update_idea(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateIdeaBody>,
) -> ApiResult<Json<Value>> {
    if let Some(content) = &body.content
        && content.trim().is_empty()
    {
        return Err(ApiError::bad_request("content 不能为空"));
    }
    let tag = body.tag.as_deref().map(parse_idea_tag).transpose()?;
    let db = lock_db(&state)?;
    let updated = db
        .update_idea(
            &id,
            body.content.as_deref().map(str::trim),
            tag,
            body.pinned,
            now_ts(),
        )
        .map_err(ApiError::internal)?;
    if !updated {
        return Err(ApiError::not_found("想法不存在"));
    }
    let idea = db
        .get_idea(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("想法不存在"))?;
    Ok(Json(idea_json(&idea)))
}

async fn delete_idea(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = lock_db(&state)?;
    if db.get_idea(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("想法不存在"));
    }
    db.delete_idea(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------- 今日任务 ----------------
/// 任务完整对象（不含同步内部字段）
fn task_json(t: &Task) -> Value {
    json!({
        "id": t.id,
        "date": t.date,
        "title": t.title,
        "priority": t.priority.label(),
        "important": t.important,
        "urgent": t.urgent,
        "pomodoro_count": t.pomodoro_count,
        "estimated_minutes": t.estimated_minutes,
        "notes": t.notes,
        "done": t.done,
        "created_ts": t.created_ts,
        "updated_ts": t.updated_ts,
    })
}

fn parse_task_priority(s: &str) -> ApiResult<TaskPriority> {
    TaskPriority::from_label(s)
        .ok_or_else(|| ApiError::bad_request("无效优先级，应为：高|中|低"))
}

#[derive(Deserialize)]
struct TasksQuery {
    date: Option<String>,
}

/// 指定日期的任务（缺省今天）：优先级 高>中>低，未完成在前，再按创建时间升序。
/// 附带项目派生提醒任务（id 以 `proj:` 前缀，不落库）。
async fn list_tasks(
    State(state): State<AppState>,
    Query(q): Query<TasksQuery>,
) -> ApiResult<Json<Value>> {
    let date = date_param(q.date)?.format("%Y-%m-%d").to_string();
    let mut tasks = lock_db(&state)?
        .tasks_on(&date)
        .map_err(ApiError::internal)?;
    let derived = lock_db(&state)?
        .project_derived_tasks(&date)
        .map_err(ApiError::internal)?;
    tasks.extend(derived);
    Ok(Json(Value::Array(tasks.iter().map(task_json).collect())))
}

#[derive(Deserialize)]
struct AddTaskBody {
    title: String,
    priority: Option<String>,
    important: Option<bool>,
    urgent: Option<bool>,
    estimated_minutes: Option<i32>,
    notes: Option<String>,
    date: Option<String>,
}

async fn add_task(
    State(state): State<AppState>,
    Json(body): Json<AddTaskBody>,
) -> ApiResult<Json<Value>> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request("title 不能为空"));
    }
    let priority = body
        .priority
        .as_deref()
        .map(parse_task_priority)
        .transpose()?
        .unwrap_or(TaskPriority::Mid);
    let date = date_param(body.date)?.format("%Y-%m-%d").to_string();
    let task = lock_db(&state)?
        .add_task(
            &date,
            title,
            priority,
            body.important.unwrap_or(false),
            body.urgent.unwrap_or(false),
            body.estimated_minutes,
            body.notes.as_deref().unwrap_or(""),
            now_ts(),
        )
        .map_err(ApiError::internal)?;
    Ok(Json(task_json(&task)))
}

#[derive(Deserialize)]
struct UpdateTaskBody {
    title: Option<String>,
    priority: Option<String>,
    important: Option<bool>,
    urgent: Option<bool>,
    pomodoro_count: Option<i32>,
    estimated_minutes: Option<Option<i32>>,
    notes: Option<String>,
    done: Option<bool>,
    date: Option<String>,
}

async fn update_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTaskBody>,
) -> ApiResult<Json<Value>> {
    if let Some(title) = &body.title
        && title.trim().is_empty()
    {
        return Err(ApiError::bad_request("title 不能为空"));
    }
    let priority = body
        .priority
        .as_deref()
        .map(parse_task_priority)
        .transpose()?;
    let date = body
        .date
        .as_deref()
        .map(|d| {
            parse_date(d)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .ok_or_else(|| ApiError::bad_request("date 格式应为 YYYY-MM-DD"))
        })
        .transpose()?;
    let db = lock_db(&state)?;
    let updated = db
        .update_task(
            &id,
            body.title.as_deref().map(str::trim),
            priority,
            body.important,
            body.urgent,
            body.pomodoro_count,
            body.estimated_minutes,
            body.notes.as_deref(),
            body.done,
            date.as_deref(),
            now_ts(),
        )
        .map_err(ApiError::internal)?;
    if !updated {
        return Err(ApiError::not_found("任务不存在"));
    }
    let task = db
        .get_task(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("任务不存在"))?;
    Ok(Json(task_json(&task)))
}

async fn delete_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let db = lock_db(&state)?;
    if db.get_task(&id).map_err(ApiError::internal)?.is_none() {
        return Err(ApiError::not_found("任务不存在"));
    }
    db.delete_task(&id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

/// 启动一个番茄钟：创建事件 + 递增任务番茄计数
async fn pomodoro_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let now = now_ts();
    let db = lock_db(&state)?;
    let task = db
        .get_task(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("任务不存在"))?;
    if db.ongoing_event().map_err(ApiError::internal)?.is_some() {
        return Err(ApiError::conflict("已有进行中事件，请先结束"));
    }
    // 创建 25 分钟的事件
    let ev = db.create_event(now).map_err(ApiError::internal)?;
    db.update_event(&ev.id, Some(&task.title), None, None, None, None)
        .map_err(ApiError::internal)?;
    db.increment_pomodoro(&id, now)
        .map_err(ApiError::internal)?;
    let task = db
        .get_task(&id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("任务不存在"))?;
    Ok(Json(json!({
        "ok": true,
        "event_id": ev.id,
        "pomodoro_count": task.pomodoro_count,
    })))
}

/// 当前进行中的事件
async fn ongoing_event(
    State(state): State<AppState>,
) -> ApiResult<Json<Value>> {
    let now = now_ts();
    let db = lock_db(&state)?;
    match db.ongoing_event().map_err(ApiError::internal)? {
        Some(ev) => Ok(Json(event_json(&ev, now))),
        None => Ok(Json(Value::Null)),
    }
}

#[derive(Deserialize)]
struct RolloverBody {
    /// 来源日期，缺省昨天
    from: Option<String>,
    /// 目标日期，缺省 from 的后一天
    to: Option<String>,
}

/// 将指定日期的未完成任务移到另一日（默认昨天→今天）。返回移动数量。
async fn rollover_tasks(
    State(state): State<AppState>,
    Json(body): Json<RolloverBody>,
) -> ApiResult<Json<Value>> {
    let now = now_ts();
    let today = today();
    let from = body
        .from
        .as_deref()
        .map(|d| {
            parse_date(d)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .ok_or_else(|| ApiError::bad_request("from 格式应为 YYYY-MM-DD"))
        })
        .transpose()?
        .unwrap_or_else(|| {
            // 缺省昨天
            (today - chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string()
        });
    let to = body
        .to
        .as_deref()
        .map(|d| {
            parse_date(d)
                .map(|d| d.format("%Y-%m-%d").to_string())
                .ok_or_else(|| ApiError::bad_request("to 格式应为 YYYY-MM-DD"))
        })
        .transpose()?
        .unwrap_or_else(|| {
            // 缺省 from 的后一天
            NaiveDate::parse_from_str(&from, "%Y-%m-%d")
                .unwrap()
                .succ_opt()
                .unwrap()
                .format("%Y-%m-%d")
                .to_string()
        });
    let count = lock_db(&state)?
        .rollover_tasks(&from, &to, now)
        .map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true, "count": count, "from": from, "to": to })))
}

// ---------------- 外部记录（/api/ext） ----------------

/// ns：^[a-z0-9][a-z0-9-]{0,31}$
fn valid_ext_ns(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// record id：^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$
fn valid_ext_id(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(*c, b'.' | b'_' | b'-'))
}

/// 记录 JSON；with_content = false 时省略 content_md（列表用）
fn ext_record_json(r: &ExtRecord, with_content: bool) -> Value {
    let props = serde_json::from_str(&r.props_json).unwrap_or(json!({}));
    let mut v = json!({
        "id": r.id,
        "title": r.title,
        "props": props,
        "created_ts": r.created_ts,
        "updated_ts": r.updated_ts,
    });
    if with_content {
        v["content_md"] = json!(r.content_md);
    }
    v
}

fn check_ext_path(ns: &str, id: Option<&str>) -> ApiResult<()> {
    if !valid_ext_ns(ns) {
        return Err(ApiError::bad_request(
            "ns 格式应为 ^[a-z0-9][a-z0-9-]{0,31}$",
        ));
    }
    if let Some(id) = id
        && !valid_ext_id(id)
    {
        return Err(ApiError::bad_request(
            "id 格式应为 ^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct UpsertExtRecordBody {
    title: Option<String>,
    /// 任意 JSON object
    props: Option<Value>,
    content_md: Option<String>,
}

/// upsert：存在则只更新出现的字段，不存在则创建（创建必须带 title）
async fn ext_upsert_record(
    State(state): State<AppState>,
    Path((ns, id)): Path<(String, String)>,
    Json(body): Json<UpsertExtRecordBody>,
) -> ApiResult<Json<Value>> {
    check_ext_path(&ns, Some(&id))?;
    if let Some(title) = &body.title
        && title.trim().is_empty()
    {
        return Err(ApiError::bad_request("title 不能为空"));
    }
    let props_json = body
        .props
        .map(|p| {
            if p.is_object() {
                Ok(p.to_string())
            } else {
                Err(ApiError::bad_request("props 应为 JSON object"))
            }
        })
        .transpose()?;
    let now = now_ts();
    let db = lock_db(&state)?;
    let record = if db
        .get_ext_record(&ns, &id)
        .map_err(ApiError::internal)?
        .is_some()
    {
        db.update_ext_record(
            &ns,
            &id,
            body.title.as_deref().map(str::trim),
            props_json.as_deref(),
            body.content_md.as_deref(),
            now,
        )
        .map_err(ApiError::internal)?;
        db.get_ext_record(&ns, &id)
            .map_err(ApiError::internal)?
            .ok_or_else(|| ApiError::not_found("记录不存在"))?
    } else {
        let title = body
            .title
            .as_deref()
            .map(str::trim)
            .ok_or_else(|| ApiError::bad_request("新建记录必须提供 title"))?;
        db.insert_ext_record(
            &ns,
            &id,
            title,
            props_json.as_deref().unwrap_or("{}"),
            body.content_md.as_deref().unwrap_or(""),
            now,
        )
        .map_err(ApiError::internal)?
    };
    Ok(Json(ext_record_json(&record, true)))
}

async fn ext_list_records(
    State(state): State<AppState>,
    Path(ns): Path<String>,
) -> ApiResult<Json<Value>> {
    check_ext_path(&ns, None)?;
    let records = lock_db(&state)?
        .ext_records(&ns)
        .map_err(ApiError::internal)?;
    Ok(Json(Value::Array(
        records.iter().map(|r| ext_record_json(r, false)).collect(),
    )))
}

async fn ext_get_record(
    State(state): State<AppState>,
    Path((ns, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    check_ext_path(&ns, Some(&id))?;
    let record = lock_db(&state)?
        .get_ext_record(&ns, &id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("记录不存在"))?;
    Ok(Json(ext_record_json(&record, true)))
}

async fn ext_delete_record(
    State(state): State<AppState>,
    Path((ns, id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    check_ext_path(&ns, Some(&id))?;
    let db = lock_db(&state)?;
    if db
        .get_ext_record(&ns, &id)
        .map_err(ApiError::internal)?
        .is_none()
    {
        return Err(ApiError::not_found("记录不存在"));
    }
    db.delete_ext_record(&ns, &id).map_err(ApiError::internal)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------------- 集成测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn test_state(configured: bool) -> AppState {
        let cfg = if configured {
            Config {
                token: "t".into(),
                parent_page_id: "p".into(),
                events_db_id: "e".into(),
                expenses_db_id: "x".into(),
                projects_db_id: "pr".into(),
                ..Config::default()
            }
        } else {
            Config::default()
        };
        AppState {
            db: Arc::new(Mutex::new(Db::in_memory().unwrap())),
            client: Arc::new(NotionClient::new("")),
            config: Arc::new(Mutex::new(cfg)),
            sync_status: Arc::new(Mutex::new(SyncStatus::default())),
        }
    }

    /// 发起一次请求，返回（状态码, 响应 JSON）
    async fn call(
        state: &AppState,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        call_auth(state, method, uri, None, body).await
    }

    /// 带 Bearer token 的请求
    async fn call_auth(
        state: &AppState,
        method: &str,
        uri: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }
        let body = match body {
            Some(v) => {
                builder = builder.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&v).unwrap())
            }
            None => Body::empty(),
        };
        let resp = router(state.clone())
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn unconfigured_gate_returns_409() {
        let state = test_state(false);
        let (code, body) = call(&state, "GET", "/api/events", None).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(body["error"], "not configured");

        // status / setup 不受限制
        let (code, body) = call(&state, "GET", "/api/status", None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["configured"], false);
        assert_eq!(body["last_sync"], Value::Null);
        assert_eq!(body["pending"], 0);

        // setup 参数校验：空 token / 非法 page_url → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/setup",
            Some(json!({"token": "", "page_url": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, body) = call(
            &state,
            "POST",
            "/api/setup",
            Some(json!({"token": "t", "page_url": "not a url"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(body["error"].is_string());
    }

    #[tokio::test]
    async fn event_start_stop_flow() {
        let state = test_state(true);

        // start → 进行中事件
        let (code, body) = call(&state, "POST", "/api/events/start", Some(json!({}))).await;
        assert_eq!(code, StatusCode::OK);
        let id = body["id"].as_str().unwrap().to_string();
        assert!(body["end_ts"].is_null());
        assert_eq!(body["content"], "");

        // 重复 start → 409
        let (code, body) = call(&state, "POST", "/api/events/start", Some(json!({}))).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(body["error"], "已有进行中事件");

        // stop 校验：空 content → 400；非法 tag → 400
        let (code, _) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "  ", "tag": "工作"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "写代码", "tag": "摸鱼"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // stop 成功
        let (code, body) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "写代码", "tag": "工作"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert!(body["end_ts"].is_number());
        assert_eq!(body["content"], "写代码");
        assert_eq!(body["tag"], "工作");

        // 重复 stop → 409
        let (code, _) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "x", "tag": "工作"})),
        )
        .await;
        assert_eq!(code, StatusCode::CONFLICT);

        // 今日列表包含该事件（按 start_ts 排序）
        let today = Local::now().format("%Y-%m-%d").to_string();
        let (code, body) = call(&state, "GET", &format!("/api/events?date={today}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], id);
        assert!(arr[0]["duration_seconds"].as_i64().unwrap() >= 0);

        // PUT 部分更新
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/events/{id}"),
            Some(json!({"content": "改需求", "tag": "学习"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "改需求");
        assert_eq!(body["tag"], "学习");

        // DELETE 后列表为空、再删 404
        let (code, _) = call(&state, "DELETE", &format!("/api/events/{id}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let (code, body) = call(&state, "GET", &format!("/api/events?date={today}"), None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 0);
        let (code, _) = call(&state, "DELETE", &format!("/api/events/{id}"), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn expenses_summary_flow() {
        let state = test_state(true);
        let now = now_ts();

        let (code, body) = call(
            &state,
            "POST",
            "/api/expenses",
            Some(json!({"item": "午饭", "amount": 25.5, "category": "餐饮", "ts": now})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["amount_cents"], 2550);
        let id = body["id"].as_str().unwrap().to_string();

        // 负数金额 / 非法分类 → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/expenses",
            Some(json!({"item": "x", "amount": -1.0, "category": "餐饮"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/expenses",
            Some(json!({"item": "x", "amount": 1.0, "category": "奶茶"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // summary：今天同时落在本周/月/年内
        let (code, body) = call(&state, "GET", "/api/expenses/summary", None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["week"], 25.5);
        assert_eq!(body["month"], 25.5);
        assert_eq!(body["year"], 25.5);

        // PUT 改金额后 summary 更新
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/expenses/{id}"),
            Some(json!({"amount": 30.0})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["amount_cents"], 3000);
        assert_eq!(body["item"], "午饭");
        let (_, body) = call(&state, "GET", "/api/expenses/summary", None).await;
        assert_eq!(body["month"], 30.0);

        // 当月列表 & 日历
        let (code, body) = call(&state, "GET", "/api/expenses", None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 1);
        let month = Local::now().format("%Y-%m").to_string();
        let today = Local::now().format("%Y-%m-%d").to_string();
        let (code, body) = call(&state, "GET", &format!("/api/calendar?month={month}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let days = body.as_array().unwrap();
        let day = days.iter().find(|d| d["date"] == today).unwrap();
        assert_eq!(day["expense"], 30.0);

        // DELETE 后 summary 归零
        let (code, _) = call(&state, "DELETE", &format!("/api/expenses/{id}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let (_, body) = call(&state, "GET", "/api/expenses/summary", None).await;
        assert_eq!(body["month"], 0.0);
    }

    #[tokio::test]
    async fn projects_crud_flow() {
        let state = test_state(true);
        let (code, body) = call(
            &state,
            "POST",
            "/api/projects",
            Some(json!({"name": "Latte", "note": "本地 Notion 客户端"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["status"], "待办");
        let id = body["id"].as_str().unwrap().to_string();

        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/projects/{id}"),
            Some(json!({"status": "已完成", "deadline_ts": null})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["status"], "已完成");
        assert_eq!(body["name"], "Latte");

        let (code, body) = call(&state, "GET", "/api/projects", None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 1);

        let (code, _) = call(&state, "DELETE", &format!("/api/projects/{id}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let (_, body) = call(&state, "GET", "/api/projects", None).await;
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn time_report_flow() {
        let state = test_state(true);
        let (code, body) = call(&state, "POST", "/api/events/start", Some(json!({}))).await;
        assert_eq!(code, StatusCode::OK);
        let id = body["id"].as_str().unwrap().to_string();
        // 把开始时间改到 1 小时前，保证有可统计的时长
        let start = now_ts() - 3600;
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/events/{id}"),
            Some(json!({"start_ts": start})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "写代码", "tag": "工作"})),
        )
        .await;

        let today = Local::now().format("%Y-%m-%d").to_string();
        let (code, body) = call(
            &state,
            "GET",
            &format!("/api/reports/time?period=day&date={today}"),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["by_tag"].as_array().unwrap().len(), 1);
        assert_eq!(body["by_tag"][0]["tag"], "工作");
        assert_eq!(body["by_tag"][0]["percent"], 100.0);
        let total = body["total_seconds"].as_i64().unwrap();
        let end = body["range_end"].as_i64().unwrap();
        assert!(total > 0 && total <= end - start);

        // 非法 period → 400
        let (code, _) = call(&state, "GET", "/api/reports/time?period=hour", None).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn manual_create_event_flow() {
        let state = test_state(true);

        // 校验：空 content / 非法 tag / end_ts <= start_ts → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({"start_ts": 1000, "end_ts": 2000, "content": "  ", "tag": "工作"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({"start_ts": 1000, "end_ts": 2000, "content": "x", "tag": "摸鱼"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({"start_ts": 2000, "end_ts": 2000, "content": "x", "tag": "工作"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 补录一条已结束事件（带 remind）
        let (code, body) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({
                "start_ts": 1000, "end_ts": 2000,
                "content": "补录会议", "tag": "工作", "remind": true,
            })),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["start_ts"], 1000);
        assert_eq!(body["end_ts"], 2000);
        assert_eq!(body["content"], "补录会议");
        assert_eq!(body["tag"], "工作");
        assert_eq!(body["remind"], true);
        assert_eq!(body["duration_seconds"], 1000);
        let id = body["id"].as_str().unwrap().to_string();

        // PUT 修改 remind
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/events/{id}"),
            Some(json!({"remind": false})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["remind"], false);
        assert_eq!(body["content"], "补录会议");

        // 计划中事件（end_ts 为 null）成功；已结束事件不冲突
        let (code, body) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({"start_ts": 3000, "end_ts": null, "content": "计划看书", "tag": "看书"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert!(body["end_ts"].is_null());
        assert_eq!(body["remind"], false); // 缺省 false

        // 已有进行中（end_ts 为 null）事件时，再建 end_ts 为 null 的 → 409
        let (code, body) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({"start_ts": 4000, "end_ts": null, "content": "又一个", "tag": "生活"})),
        )
        .await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(body["error"], "已有进行中事件");

        // 但补录已结束事件不受限制
        let (code, _) = call(
            &state,
            "POST",
            "/api/events",
            Some(json!({"start_ts": 5000, "end_ts": 6000, "content": "补录2", "tag": "运动"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
    }

    #[tokio::test]
    async fn ai_recognize_route_validation() {
        let state = test_state(true);
        // 指向一个必然连不上的地址，验证 502 映射
        state.config.lock().unwrap().ai.proxy_base = "http://127.0.0.1:1".into();

        // 校验：非法 media_type / 空 image / 非法 date → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/ai/recognize-events",
            Some(json!({"image_base64": "QUJD", "media_type": "image/gif", "date": "2024-08-01"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/ai/recognize-events",
            Some(json!({"image_base64": "", "media_type": "image/png", "date": "2024-08-01"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/ai/recognize-events",
            Some(json!({"image_base64": "QUJD", "media_type": "image/png", "date": "2024/08/01"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 代理连不上 → 502 AI 服务不可用
        let (code, body) = call(
            &state,
            "POST",
            "/api/ai/recognize-events",
            Some(json!({"image_base64": "QUJD", "media_type": "image/png", "date": "2024-08-01"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert!(body["error"].as_str().unwrap().starts_with("AI 服务不可用"));
    }

    #[tokio::test]
    async fn notes_crud_and_tree_flow() {
        let state = test_state(true);

        // 根目录 + 目录下的文档
        let (code, body) = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": null, "kind": "dir", "title": "根目录"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["kind"], "dir");
        assert!(body["parent_id"].is_null());
        assert_eq!(body["content_md"], "");
        assert!(body["created_ts"].is_number() && body["updated_ts"].is_number());
        let dir_id = body["id"].as_str().unwrap().to_string();

        let (code, body) = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": dir_id, "kind": "doc", "title": "文档", "content_md": "# 你好"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content_md"], "# 你好");
        let doc_id = body["id"].as_str().unwrap().to_string();

        // 树：根 → 根目录 → 文档
        let (code, body) = call(&state, "GET", "/api/notes/tree", None).await;
        assert_eq!(code, StatusCode::OK);
        let tree = body.as_array().unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0]["id"], dir_id);
        assert_eq!(tree[0]["title"], "根目录");
        assert!(tree[0]["created_ts"].is_number() && tree[0]["updated_ts"].is_number());
        let children = tree[0]["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["id"], doc_id);
        assert!(children[0]["children"].as_array().unwrap().is_empty());

        // 单条查询；不存在 → 404
        let (code, body) = call(&state, "GET", &format!("/api/notes/{doc_id}"), None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["title"], "文档");
        assert_eq!(body["kind"], "doc");
        assert!(body["updated_ts"].is_number());
        let (code, _) = call(&state, "GET", "/api/notes/nope", None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);

        // PUT 改标题/内容
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/notes/{doc_id}"),
            Some(json!({"title": "改名", "content_md": "正文"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["title"], "改名");
        assert_eq!(body["content_md"], "正文");

        // PUT 显式 null → 移到根
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/notes/{doc_id}"),
            Some(json!({"parent_id": null})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert!(body["parent_id"].is_null());

        // DELETE 目录 → 递归删除（doc 已移出，不受影响）
        let sub = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": dir_id, "kind": "doc", "title": "子文档"})),
        )
        .await
        .1["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (code, _) = call(&state, "DELETE", &format!("/api/notes/{dir_id}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let (code, _) = call(&state, "GET", &format!("/api/notes/{sub}"), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        let (code, _) = call(&state, "DELETE", &format!("/api/notes/{dir_id}"), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        let (_, body) = call(&state, "GET", "/api/notes/tree", None).await;
        let tree = body.as_array().unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0]["id"], doc_id);
    }

    #[tokio::test]
    async fn notes_validation_errors() {
        let state = test_state(true);

        // 空 title / 非法 kind / 父不存在 → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": null, "kind": "dir", "title": "  "})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": null, "kind": "file", "title": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": "ghost", "kind": "doc", "title": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 父是 doc → 400
        let doc = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": null, "kind": "doc", "title": "文档"})),
        )
        .await
        .1["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (code, body) = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": doc, "kind": "doc", "title": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "父节点必须是目录");

        // 防环：A → B，A 不能移到 B 下，也不能移到自己下
        let a = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": null, "kind": "dir", "title": "A"})),
        )
        .await
        .1["id"]
            .as_str()
            .unwrap()
            .to_string();
        let b = call(
            &state,
            "POST",
            "/api/notes",
            Some(json!({"parent_id": a, "kind": "dir", "title": "B"})),
        )
        .await
        .1["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/notes/{a}"),
            Some(json!({"parent_id": b})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/notes/{a}"),
            Some(json!({"parent_id": a})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // PUT 空 title → 400；PUT 不存在 → 404
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/notes/{a}"),
            Some(json!({"title": " "})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            "/api/notes/nope",
            Some(json!({"title": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn ideas_crud_flow() {
        let state = test_state(true);

        // 校验：空 content / 非法 tag → 400
        let (code, _) = call(&state, "POST", "/api/ideas", Some(json!({"content": "  "}))).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/ideas",
            Some(json!({"content": "x", "tag": "摸鱼"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 创建：tag 缺省「灵感」，pinned 缺省 false
        let (code, body) = call(
            &state,
            "POST",
            "/api/ideas",
            Some(json!({"content": "  想法一  "})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "想法一");
        assert_eq!(body["tag"], "灵感");
        assert_eq!(body["pinned"], false);
        assert!(body["created_ts"].is_number() && body["updated_ts"].is_number());
        let id1 = body["id"].as_str().unwrap().to_string();
        let created1 = body["created_ts"].as_i64().unwrap();

        let (_, body) = call(
            &state,
            "POST",
            "/api/ideas",
            Some(json!({"content": "想法二", "tag": "待办", "pinned": true})),
        )
        .await;
        assert_eq!(body["tag"], "待办");
        assert_eq!(body["pinned"], true);
        let id2 = body["id"].as_str().unwrap().to_string();

        // 列表：置顶优先；/api/status 的 pending 计入 ideas
        let (code, body) = call(&state, "GET", "/api/ideas", None).await;
        assert_eq!(code, StatusCode::OK);
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], id2);
        assert_eq!(arr[1]["id"], id1);
        let (_, body) = call(&state, "GET", "/api/status", None).await;
        assert_eq!(body["pending"], 2);

        // tag 过滤；非法 tag → 400
        let (code, body) = call(&state, "GET", "/api/ideas?tag=待办", None).await;
        assert_eq!(code, StatusCode::OK);
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], id2);
        let (code, _) = call(&state, "GET", "/api/ideas?tag=摸鱼", None).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // PUT 部分更新：只改 pinned，其余不变
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/ideas/{id1}"),
            Some(json!({"pinned": true})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "想法一");
        assert_eq!(body["tag"], "灵感");
        assert_eq!(body["pinned"], true);
        assert_eq!(body["created_ts"], created1);
        assert!(body["updated_ts"].as_i64().unwrap() >= created1);

        // PUT 改内容与标签；空 content / 非法 tag → 400；不存在 → 404
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/ideas/{id1}"),
            Some(json!({"content": "改名", "tag": "读书"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "改名");
        assert_eq!(body["tag"], "读书");
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/ideas/{id1}"),
            Some(json!({"content": " "})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/ideas/{id1}"),
            Some(json!({"tag": "摸鱼"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            "/api/ideas/nope",
            Some(json!({"content": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND);

        // DELETE → ok；再删 404；列表为空
        let (code, body) = call(&state, "DELETE", &format!("/api/ideas/{id1}"), None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (code, _) = call(&state, "DELETE", &format!("/api/ideas/{id1}"), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        let (code, _) = call(&state, "DELETE", &format!("/api/ideas/{id2}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let (_, body) = call(&state, "GET", "/api/ideas", None).await;
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn tasks_crud_flow() {
        let state = test_state(true);

        // 校验：空 title / 非法 priority / 非法 date → 400
        let (code, _) = call(&state, "POST", "/api/tasks", Some(json!({"title": "  "}))).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "x", "priority": "急"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "x", "date": "2024/08/01"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // 创建：priority 缺省「中」，date 缺省今天；title 去空白
        let (code, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "  写周报  "})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["title"], "写周报");
        assert_eq!(body["priority"], "中");
        assert_eq!(body["done"], false);
        let today = Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(body["date"], today);
        assert!(body["created_ts"].is_number() && body["updated_ts"].is_number());
        let id_mid = body["id"].as_str().unwrap().to_string();
        let created_mid = body["created_ts"].as_i64().unwrap();

        let (_, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "修 bug", "priority": "高"})),
        )
        .await;
        let id_high = body["id"].as_str().unwrap().to_string();
        let (_, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "明天的活", "priority": "低", "date": "2099-01-01"})),
        )
        .await;
        let id_tomorrow = body["id"].as_str().unwrap().to_string();

        // 列表：按日期过滤，高优先级在前；pending 计入 tasks
        let (code, body) = call(&state, "GET", "/api/tasks", None).await;
        assert_eq!(code, StatusCode::OK);
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], id_high);
        assert_eq!(arr[1]["id"], id_mid);
        let (_, body) = call(&state, "GET", "/api/tasks?date=2099-01-01", None).await;
        let arr = body.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], id_tomorrow);
        let (code, _) = call(&state, "GET", "/api/tasks?date=20990101", None).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (_, body) = call(&state, "GET", "/api/status", None).await;
        assert_eq!(body["pending"], 3);

        // 完成高优任务后排到未完成的中优之后
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/tasks/{id_high}"),
            Some(json!({"done": true})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["done"], true);
        assert_eq!(body["title"], "修 bug");
        let (_, body) = call(&state, "GET", "/api/tasks", None).await;
        let arr = body.as_array().unwrap();
        assert_eq!(arr[0]["id"], id_mid);
        assert_eq!(arr[1]["id"], id_high);

        // PUT 部分更新：改标题/优先级/日期，created_ts 不变，updated_ts 刷新
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/tasks/{id_mid}"),
            Some(json!({"title": "写月报", "priority": "低", "date": "2099-01-02"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["title"], "写月报");
        assert_eq!(body["priority"], "低");
        assert_eq!(body["date"], "2099-01-02");
        assert_eq!(body["created_ts"], created_mid);
        assert!(body["updated_ts"].as_i64().unwrap() >= created_mid);
        // 已移出今天
        let (_, body) = call(&state, "GET", "/api/tasks", None).await;
        assert_eq!(body.as_array().unwrap().len(), 1);

        // PUT 校验：空 title / 非法 priority / 非法 date → 400；不存在 → 404
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/tasks/{id_mid}"),
            Some(json!({"title": " "})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/tasks/{id_mid}"),
            Some(json!({"priority": "急"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            &format!("/api/tasks/{id_mid}"),
            Some(json!({"date": "昨天"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "PUT",
            "/api/tasks/nope",
            Some(json!({"title": "x"})),
        )
        .await;
        assert_eq!(code, StatusCode::NOT_FOUND);

        // DELETE → ok；再删 404；列表移除
        let (code, body) = call(&state, "DELETE", &format!("/api/tasks/{id_high}"), None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (code, _) = call(&state, "DELETE", &format!("/api/tasks/{id_high}"), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        let (_, body) = call(&state, "GET", "/api/tasks", None).await;
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn tasks_quadrant_and_derived_flow() {
        let state = test_state(true);

        // 创建带重要/紧急的任务
        let (code, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "重要且紧急", "important": true, "urgent": true})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["important"], true);
        assert_eq!(body["urgent"], true);
        let id = body["id"].as_str().unwrap().to_string();

        // 缺省两个开关为 false
        let (_, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "普通任务"})),
        )
        .await;
        assert_eq!(body["important"], false);
        assert_eq!(body["urgent"], false);

        // 部分更新只改紧急
        let (code, body) = call(
            &state,
            "PUT",
            &format!("/api/tasks/{id}"),
            Some(json!({"urgent": false, "important": true})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["important"], true);
        assert_eq!(body["urgent"], false);

        // 项目派生提醒：截止今天
        let today = Local::now().format("%Y-%m-%d").to_string();
        let today_ts = Local::now().date_naive()
            .and_hms_opt(12, 0, 0).unwrap()
            .and_utc().timestamp();
        let (_, body) = call(
            &state,
            "POST",
            "/api/projects",
            Some(json!({"name": "发版", "status": "进行中", "deadline_ts": today_ts})),
        )
        .await;
        let proj_id = body["id"].as_str().unwrap().to_string();

        let (code, body) = call(&state, "GET", &format!("/api/tasks?date={today}"), None).await;
        assert_eq!(code, StatusCode::OK);
        let arr = body.as_array().unwrap();
        let derived: Vec<_> = arr.iter().filter(|t| t["id"].as_str().is_some_and(|s| s.starts_with("proj:"))).collect();
        assert_eq!(derived.len(), 1);
        assert_eq!(derived[0]["title"], "📌 发版 截止");
        assert_eq!(derived[0]["important"], true);
        assert_eq!(derived[0]["urgent"], true);
        assert_eq!(derived[0]["date"], today);
        // 派生任务 id（proj: 前缀）不在库中，删除 → 404
        let (code, _) = call(&state, "DELETE", &format!("/api/tasks/proj:{proj_id}"), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tasks_rollover_flow() {
        let state = test_state(true);

        // 昨天一笔未完成 + 一笔已完成
        let yesterday = (Local::now().date_naive() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let (code, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "昨天的活", "priority": "高", "date": yesterday})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let id_undo = body["id"].as_str().unwrap().to_string();
        let (code, body) = call(
            &state,
            "POST",
            "/api/tasks",
            Some(json!({"title": "昨天做完的", "date": yesterday})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let id_done = body["id"].as_str().unwrap().to_string();
        call(
            &state,
            "PUT",
            &format!("/api/tasks/{id_done}"),
            Some(json!({"done": true})),
        )
        .await;

        // 缺省：昨天 → 今天，只移未完成
        let (code, body) = call(&state, "POST", "/api/tasks/rollover", Some(json!({}))).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["count"], 1);
        let today = Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(body["from"], yesterday);
        assert_eq!(body["to"], today);
        let (_, body) = call(&state, "GET", &format!("/api/tasks?date={today}"), None).await;
        let arr = body.as_array().unwrap();
        assert!(arr.iter().any(|t| t["id"] == id_undo));
        assert!(!arr.iter().any(|t| t["id"] == id_done));

        // 再次执行：昨天已无未完成 → count 0
        let (_, body) = call(&state, "POST", "/api/tasks/rollover", Some(json!({}))).await;
        assert_eq!(body["count"], 0);

        // 显式 from/to 校验：非法日期 → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/tasks/rollover",
            Some(json!({"from": "bad"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = call(
            &state,
            "POST",
            "/api/tasks/rollover",
            Some(json!({"to": "bad"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn event_start_prefill_and_stop_optional() {
        let state = test_state(true);

        // start 带非法 tag → 400
        let (code, _) = call(
            &state,
            "POST",
            "/api/events/start",
            Some(json!({"content": "x", "tag": "摸鱼"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // start 预填 content + tag
        let (code, body) = call(
            &state,
            "POST",
            "/api/events/start",
            Some(json!({"content": "  写报告  ", "tag": "工作"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "写报告");
        assert_eq!(body["tag"], "工作");
        let id = body["id"].as_str().unwrap().to_string();

        // stop 空 body：保留预填的 content 与 tag
        let (code, body) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert!(body["end_ts"].is_number());
        assert_eq!(body["content"], "写报告");
        assert_eq!(body["tag"], "工作");

        // start 无 body（不带 content-type）也能工作；stop 只给 tag：content 为空 → 400
        let (code, body) = call(&state, "POST", "/api/events/start", None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "");
        assert_eq!(body["tag"], Value::Null);
        let id = body["id"].as_str().unwrap().to_string();
        let (code, _) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"tag": "学习"})),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_REQUEST);

        // stop 只给 content：tag 默认「生活」
        let (code, body) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "散步"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "散步");
        assert_eq!(body["tag"], "生活");

        // start 只预填 content；stop 只给 tag：content 保留
        let (_, body) = call(
            &state,
            "POST",
            "/api/events/start",
            Some(json!({"content": "看书"})),
        )
        .await;
        let id = body["id"].as_str().unwrap().to_string();
        let (code, body) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"tag": "看书"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content"], "看书");
        assert_eq!(body["tag"], "看书");

        // stop 空字符串 tag → 默认「生活」
        let (_, body) = call(&state, "POST", "/api/events/start", Some(json!({}))).await;
        let id = body["id"].as_str().unwrap().to_string();
        let (code, body) = call(
            &state,
            "POST",
            &format!("/api/events/{id}/stop"),
            Some(json!({"content": "杂事", "tag": "  "})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["tag"], "生活");
    }

    #[tokio::test]
    async fn ext_auth_flow() {
        let state = test_state(true);

        // api_token 为空（未生成）→ 409
        let (code, body) = call(&state, "GET", "/api/ext/agents/records", None).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert_eq!(body["error"], "not configured");

        state.config.lock().unwrap().api_token = "secret".into();
        // 无头 / 错误 token → 401
        let (code, body) = call(&state, "GET", "/api/ext/agents/records", None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");
        let (code, _) = call_auth(
            &state,
            "GET",
            "/api/ext/agents/records",
            Some("wrong"),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
        // 正确 token → 200 空列表
        let (code, body) = call_auth(
            &state,
            "GET",
            "/api/ext/agents/records",
            Some("secret"),
            None,
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body.as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn ext_path_and_body_validation() {
        let state = test_state(true);
        state.config.lock().unwrap().api_token = "secret".into();
        let put = |ns: &str, id: &str, body: Value| {
            let state = state.clone();
            let uri = format!("/api/ext/{ns}/records/{id}");
            async move { call_auth(&state, "PUT", &uri, Some("secret"), Some(body)).await }
        };
        // 非法 ns：大写 / 下划线 / 超长（33 字符）→ 400
        let (code, _) = put("Agents", "r1", json!({"title": "x"})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = put("my_ns", "r1", json!({"title": "x"})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = put(&"a".repeat(33), "r1", json!({"title": "x"})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        // 非法 id：首字符非字母数字 / 超长（65 字符）→ 400
        let (code, _) = put("agents", ".hidden", json!({"title": "x"})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = put("agents", &"a".repeat(65), json!({"title": "x"})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        // props 非 object → 400；新建缺 title → 400；空 title → 400
        let (code, body) = put("agents", "r1", json!({"title": "x", "props": [1, 2]})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "props 应为 JSON object");
        let (code, _) = put("agents", "r1", json!({"props": {}})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        let (code, _) = put("agents", "r1", json!({"title": "  "})).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        // 合法边界：ns 含数字和连字符、id 含 ._- 均可
        let (code, _) = put("app-2", "Task.1_2-3", json!({"title": "x"})).await;
        assert_eq!(code, StatusCode::OK);
    }

    #[tokio::test]
    async fn ext_upsert_list_get_delete_flow() {
        let state = test_state(true);
        state.config.lock().unwrap().api_token = "secret".into();
        let auth = |method: &str, uri: String, body: Option<Value>| {
            let state = state.clone();
            let method = method.to_string();
            async move { call_auth(&state, &method, &uri, Some("secret"), body).await }
        };

        // 创建
        let (code, body) = auth(
            "PUT",
            "/api/ext/agents/records/task-1".into(),
            Some(json!({"title": "任务一", "props": {"status": "todo"}, "content_md": "# 详情"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["id"], "task-1");
        assert_eq!(body["title"], "任务一");
        assert_eq!(body["props"]["status"], "todo");
        assert_eq!(body["content_md"], "# 详情");
        assert!(body["created_ts"].is_number() && body["updated_ts"].is_number());
        let created_ts = body["created_ts"].as_i64().unwrap();

        // 幂等：重复全量 PUT，created_ts 不变
        let (code, body) = auth(
            "PUT",
            "/api/ext/agents/records/task-1".into(),
            Some(json!({"title": "任务一", "props": {"status": "todo"}, "content_md": "# 详情"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["created_ts"], created_ts);

        // 部分更新：只改 title，props/content_md 保留
        let (code, body) = auth(
            "PUT",
            "/api/ext/agents/records/task-1".into(),
            Some(json!({"title": "任务一（改）"})),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["title"], "任务一（改）");
        assert_eq!(body["props"]["status"], "todo");
        assert_eq!(body["content_md"], "# 详情");

        // 再建一条，列表按 updated_ts 倒序、不含 content_md
        auth(
            "PUT",
            "/api/ext/agents/records/task-2".into(),
            Some(json!({"title": "任务二"})),
        )
        .await;
        let (code, body) = auth("GET", "/api/ext/agents/records".into(), None).await;
        assert_eq!(code, StatusCode::OK);
        let list = body.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["id"], "task-2");
        assert_eq!(list[1]["id"], "task-1");
        assert!(list[0].get("content_md").is_none());
        // 命名空间隔离
        let (_, body) = auth("GET", "/api/ext/other/records".into(), None).await;
        assert_eq!(body.as_array().unwrap().len(), 0);

        // 单条查询含 content_md；不存在 → 404
        let (code, body) = auth("GET", "/api/ext/agents/records/task-1".into(), None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["content_md"], "# 详情");
        let (code, _) = auth("GET", "/api/ext/agents/records/nope".into(), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);

        // 删除：ok → 再查 404 → 再删 404
        let (code, body) = auth("DELETE", "/api/ext/agents/records/task-1".into(), None).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(body["ok"], true);
        let (code, _) = auth("GET", "/api/ext/agents/records/task-1".into(), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        let (code, _) = auth("DELETE", "/api/ext/agents/records/task-1".into(), None).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
    }
}
