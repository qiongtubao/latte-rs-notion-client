# Latte

本地 Notion 客户端（Rust Web 应用）：每日时间碎片管理、金钱管理、日历、项目管理。

架构：**Rust(axum) 后端 + Vue 3 前端**，浏览器使用。数据流为**本地优先** —— 所有操作立即写入本地 SQLite（标记 `dirty`），后台任务每 30 秒把变更推送/更新到 Notion，失败自动重试，离线可正常使用。

## 功能

- **时间碎片**：一键开始/结束计时，结束时填写「做了什么 + 标签」（工作/运动/生活/学习/看书），进行中事件实时计时；日/周/月/年报表按标签汇总时长与占比。
- **金钱**：记录消费（事项/金额/分类：餐饮/交通/购物/娱乐/其他），顶部常驻本周/本月/本年汇总。
- **日历**：月视图（周一起始），每格显示当日碎片总时长与消费额；点击日期打开 **24 小时时间线**：碎片按真实起止渲染为色块（按标签着色），进行中事件脉冲延伸，今天有当前时刻指示线；点击时间线空白处可直接**补录/安排事件**（支持「到点提醒」，浏览器系统通知）；**图片识别**：上传日程截图，AI 自动抽取事件，可编辑确认后批量入库。
- **项目**：看板视图（暂存/待办/进行中/已完成/暂停 5 列），拖拽卡片改状态；甘特图视图按开始/截止日期展示时间跨度，未排期项目虚线占位。
- **好想法**：快速输入（Ctrl+Enter 收集）+ 瀑布流卡片墙，支持置顶、标签（灵感/待办/读书/问题/其他）筛选、编辑/删除；同步到 Notion「好想法」database（首次有条目时自动创建）。
- **知识库**：总览页卡片展示所有知识库 + 创建入口；进入后左侧目录树（无限层级、拖拽移动）、右侧 Markdown 显示/编辑（Ctrl+S 保存）。Notion 端镜像为「📚 知识库」根页下的嵌套子页面。
- **对外 API**：`/api/ext/{命名空间}/records` 通用记录接口（Bearer token 鉴权），供其他本地应用（如 latte-rs-agents）把任务/文档数据经 Latte 同步进 Notion，详见下文。
- **Notion 同步**：首次配置时自动在指定父页面下创建 3 个 database + 知识库根页，此后所有增删改自动同步（删除 = Notion 端归档；`remind` 提醒标记仅存本地不同步）。顶栏显示同步状态，可手动「立即同步」。

## AI 图片识别（可选）

图片识别依赖本地的 [latte-model-proxy](../latte-rs-model-router)（OpenAI 兼容代理，支持视觉模型路由）。先启动代理：

```bash
cd ../latte-rs-model-router/latte_project_debug
cargo run -p latte-model-proxy --release -- --config=./proxy.toml   # 监听 127.0.0.1:16434
```

用哪个视觉模型由代理的 `models.d/*.toml` 和 `proxy.toml` 的 pool 决定，Latte 默认向 `proxy-default` 发请求。如需改代理地址或指定模型，编辑 `~/.config/latte/config.toml`：

```toml
[ai]
proxy_base = "http://127.0.0.1:16434"
model = "proxy-default"   # 也可填具体模型 id，proxy 直转
api_key = ""              # 代理有 auth 时填
```

## 构建与运行

```bash
# 1. 构建前端（需要 node + pnpm）
cd frontend && pnpm install && pnpm build && cd ..

# 2. 构建并启动后端（需要 Rust 1.88+，edition 2024）
cargo run --release
```

启动后打开 <http://127.0.0.1:3210>。后端直接托管 `frontend/dist`，无需单独起前端服务。

前端开发调试（热更新）：

```bash
cd frontend && pnpm dev    # vite dev server，/api 自动代理到 127.0.0.1:3210
```

## Notion 配置（首次使用）

首次打开页面会进入配置向导，按页面提示操作：

1. 打开 <https://www.notion.so/profile/integrations>，点击「+ New integration」，名称随意（如 `latte`），复制 **Internal Integration Secret**（`ntn_` 开头）。
2. 在 Notion 新建一个页面（如「Latte 数据」），点页面右上角 `···` → **Connect to / 添加连接** → 选择刚创建的 integration。
3. 在向导页粘贴 token 和该页面的 URL，提交后程序自动在页面下创建 3 个 database：

   | Database | 字段 |
   |---|---|
   | 时间碎片 | 名称(title)、开始(date)、结束(date)、内容(rich_text)、标签(select：工作/运动/生活/学习/看书) |
   | 金钱记录 | 事项(title)、金额(number)、时间(date)、分类(select：餐饮/交通/购物/娱乐/其他) |
   | 项目管理 | 名称(title)、状态(select：暂存/待办/进行中/已完成/暂停)、开始(date)、截止(date)、备注(rich_text) |

配置文件写入 `~/.config/latte/config.toml`（token、parent_page_id、3 个 database_id），本地数据库为 `~/.config/latte/latte.db`。配置成功后无需重启即生效。

如需重新配置，删除 `~/.config/latte/config.toml` 再刷新页面即可。

## API 概览（前缀 `/api`，全部 JSON）

| 端点 | 说明 |
|---|---|
| `GET /api/status` · `POST /api/setup` | 配置状态 / 初始化（token + page_url） |
| `GET /api/events?date=` · `POST /api/events`（手动创建，含 remind）· `POST /api/events/start` · `POST /api/events/:id/stop` · `PUT/DELETE /api/events/:id` | 时间碎片 |
| `GET /api/reports/time?period=day\|week\|month\|year&date=` | 时长报表 |
| `POST /api/ai/recognize-events` | 图片识别事件（需 latte-model-proxy） |
| `GET/POST /api/expenses` · `PUT/DELETE /api/expenses/:id` · `GET /api/expenses/summary` | 金钱（提交金额单位为元，返回 `amount_cents`） |
| `GET /api/calendar?month=` · `GET /api/calendar/day?date=` | 日历 |
| `GET/POST /api/projects` · `PUT/DELETE /api/projects/:id` | 项目 |
| `GET/POST /api/ideas` · `PUT/DELETE /api/ideas/:id` | 好想法卡片 |
| `GET /api/notes/tree` · `POST /api/notes` · `GET/PUT/DELETE /api/notes/:id` | 知识库（无限层级目录/文档） |
| `PUT /api/ext/{ns}/records/{id}` · `GET /api/ext/{ns}/records(/:id)` · `DELETE /api/ext/{ns}/records/{id}` | 通用记录（需 Bearer token） |
| `POST /api/sync` | 立即同步 |

## 对外 API（/api/ext）

供其他本地应用（如 latte-rs-agents）把数据经 Latte 同步进 Notion。鉴权：`Authorization: Bearer <api_token>`，token 首次启动自动生成并写入 `~/.config/latte/config.toml`。

- `PUT /api/ext/{ns}/records/{id}` — 幂等 upsert，body `{"title": "...", "props": {...任意 JSON}, "content_md": "..."}`；ns 形如 `agents`（小写字母/数字/连字符），id 由客户端自定（如任务 id）
- `GET /api/ext/{ns}/records` — 列表（不含 content_md，按更新时间倒序）
- `GET /api/ext/{ns}/records/{id}` — 完整记录
- `DELETE /api/ext/{ns}/records/{id}`

每个命名空间自动在 Notion 父页面下建「📦 {ns}」根页，记录镜像为其下子页面（props 以 JSON 代码块呈现，content_md 转 Notion blocks），复用本地优先 + dirty 同步与失败重试。

未配置时除 status/setup 外一律返回 409 `{"error":"not configured"}`；错误统一 `{"error": "..."}`。

## 同步机制

- 本地写操作立即生效并标记 `dirty = 1`。
- 后台 tokio 任务每 30 秒扫描 dirty 行：`notion_page_id` 为空 → POST 新建并回填 id；已有 → PATCH 更新；`deleted = 1` → 远端归档（`archived: true`）后删除本地行。
- 单行失败保留 dirty，下轮自动重试；遵守 Notion 限速（请求间隔 ≥ 350ms，429 按 `Retry-After` 退避，最多 3 次）。
- 顶栏显示最近同步时间 / 待同步条数 / 错误信息；退出（Ctrl-C）前做最后一次冲刷（最多 5 秒）。

## 开发与测试

```bash
cargo test                  # 37 个测试：report 聚合、db CRUD、notion 请求体、API 集成
cargo clippy --all-targets -- -D warnings
cargo fmt
cd frontend && pnpm build   # 前端构建
```

代码结构：

```
src/
  main.rs     入口：启动 axum 服务、静态托管、后台同步任务
  lib.rs      库入口（api/config/db/models/notion/report/sync）
  api.rs      REST API 路由 + 集成测试
  config.rs   ~/.config/latte/config.toml 读写、page id 解析
  models.rs   Event / Expense / Project / Tag / Category / ProjectStatus
  db.rs       SQLite schema + CRUD（dirty 软同步标记）
  notion.rs   Notion API 客户端 + 请求体构造纯函数
  sync.rs     后台同步循环（每 30s 冲刷 dirty 行）
  report.rs   日/周/月/年聚合（纯函数）
frontend/
  src/api.js            axios 封装
  src/App.vue           顶栏 + Tab 导航 + 同步状态
  src/views/            SetupView / TimeView / MoneyView / CalendarView / ProjectsView
```
