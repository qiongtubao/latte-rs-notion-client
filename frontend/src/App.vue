<template>
  <div v-if="loading" class="boot">
    <el-icon class="is-loading" :size="32"><Loading /></el-icon>
  </div>

  <SetupView v-else-if="!configured" @done="onSetupDone" />

  <div v-else class="app-root">
    <!-- 极简顶栏 -->
    <header class="topbar">
      <span class="brand">☕ Latte</span>
      <span class="top-date">{{ todayStr }}</span>
      <div class="top-right">
        <el-button size="small" @click="openQuickEntry">⚡ 快录<span class="kbd-hint">Ctrl+K</span></el-button>
        <!-- 全局搜索：远程子串匹配，选中后打开对应面板（知识库会定位到文档） -->
        <el-select
          v-model="searchPick"
          filterable
          remote
          clearable
          :remote-method="doSearch"
          :loading="searching"
          placeholder="搜索…"
          size="small"
          class="top-search"
          @change="onSearchPick"
        >
          <el-option-group v-for="g in searchGroups" :key="g.kind" :label="g.label">
            <el-option
              v-for="h in g.items"
              :key="h.kind + ':' + h.id"
              :value="h.kind + ':' + h.id"
              :label="h.title || '(无标题)'"
            >
              <div class="srch-opt">
                <span class="srch-title">{{ h.title || '(无标题)' }}</span>
                <span class="srch-snippet">{{ h.snippet }}</span>
              </div>
            </el-option>
          </el-option-group>
        </el-select>
        <span class="sync-info">
          <template v-if="status.last_error">
            <span class="sync-error">{{ status.last_error }}</span>
          </template>
          <template v-else>
            同步 {{ status.last_sync ? dayjs(status.last_sync).format('HH:mm') : '从未' }}
          </template>
          <el-badge v-if="status.pending > 0" :value="status.pending" type="warning" class="pending-badge" />
        </span>
        <el-button size="small" circle :type="remindOn ? 'primary' : 'default'" @click="toggleRemind">
          <el-icon><BellFilled v-if="remindOn" /><Bell v-else /></el-icon>
        </el-button>
        <el-button size="small" :loading="syncing" @click="doSync">同步</el-button>
        <el-button size="small" :loading="pulling" @click="doPull">拉取</el-button>
        <el-button size="small" type="danger" plain :loading="resetting" @click="doReset">重置</el-button>
        <el-dropdown trigger="click" @command="downloadExport">
          <el-button size="small">导出</el-button>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="json">全量备份（JSON）</el-dropdown-item>
              <el-dropdown-item command="events">事件（CSV）</el-dropdown-item>
              <el-dropdown-item command="expenses">消费（CSV）</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </header>

    <!-- 主区域：干净背景（其他功能从浮动按钮进入） -->
    <main class="main-area">
      <div class="empty-bg">
        <div class="empty-icon">☕</div>
        <p class="empty-hint">点击浮动按钮开始</p>
      </div>
    </main>

    <!-- 今日任务列表：默认隐藏，点击「今日任务」按钮随子按钮一起显示/隐藏，跟随按钮拖动 -->
    <!-- 各 dock 统一挂 panelRefreshKey：拉取/重置/手动刷新后重建，避免停在旧数据或空态 -->
    <aside v-if="dockOpen.today" class="today-dock" :style="todayDockStyle">
      <TodayTaskList :key="panelRefreshKey" />
    </aside>

    <!-- 金钱列表：同上，跟随「金钱」按钮 -->
    <aside v-if="dockOpen.money" class="today-dock" :style="moneyDockStyle">
      <MoneyExpenseList :key="panelRefreshKey" />
    </aside>

    <!-- 日历列表：同上，跟随「日历」按钮 -->
    <aside v-if="dockOpen.calendar" class="today-dock" :style="calendarDockStyle">
      <CalendarDayList :key="panelRefreshKey" />
    </aside>

    <!-- 项目列表/甘特图：同上，跟随「项目」按钮 -->
    <aside v-if="dockOpen.projects" class="today-dock dock-wide" :style="projectsDockStyle">
      <ProjectList :key="panelRefreshKey" />
    </aside>

    <!-- 知识库：编辑器式布局（左目录右内容），更宽 -->
    <aside v-if="dockOpen.notes" class="today-dock dock-notes" :style="notesDockStyle">
      <NoteList :key="panelRefreshKey" />
    </aside>

    <!-- 好想法列表：同上，跟随「好想法」按钮 -->
    <aside v-if="dockOpen.ideas" class="today-dock" :style="ideasDockStyle">
      <IdeaList :key="panelRefreshKey" />
    </aside>

    <!-- 浮动按钮层 -->
    <FloatingButton
      v-for="fb in floatButtons"
      :key="fb.key"
      :id="fb.key"
      :icon="fb.icon"
      :label="fb.label"
      :color="fb.color"
      :x="fb.x"
      :y="fb.y"
      :subs="fb.subs"
      :emphasized="activePanel === fb.key"
      :timing="fb.key === 'today' && timing"
      @open="onFloatOpen(fb)"
      @sub="onFloatSub(fb, $event)"
      @toggle="onFloatToggle(fb, $event)"
      @move="onFloatMove(fb, $event)"
    />

    <!-- AI 快速录入：一句话 → 解析预览 → 确认创建 -->
    <el-dialog v-model="qeDialog" title="⚡ 快速录入" width="460px">
      <el-input
        v-model="qeText"
        placeholder="午饭 25 / 明天下午3点开会提醒我 / 灵感：……"
        @keyup.enter="qeParse"
      >
        <template #append>
          <el-button :loading="qeParsing" @click="qeParse">解析</el-button>
        </template>
      </el-input>
      <div v-if="qeDraft" class="qe-preview">
        <div class="qe-type">{{ qeTypeLabel }}</div>
        <div v-for="f in qeFields" :key="f.k" class="qe-field">
          <span class="qe-k">{{ f.k }}</span>
          <span>{{ f.v }}</span>
        </div>
      </div>
      <template #footer>
        <el-button @click="qeDialog = false">取消</el-button>
        <el-button type="primary" :disabled="!qeDraft" :loading="qeSaving" @click="qeConfirm">确认创建</el-button>
      </template>
    </el-dialog>

    <!-- 弹出面板（浮于内容之上） -->
    <Transition name="panel-fade">
      <div v-if="activePanel" class="panel-overlay" @click="closePanel()" />
    </Transition>
    <Transition name="panel-slide">
      <aside v-if="activePanel" class="side-panel" :class="{ 'side-panel-wide': activePanel === 'notes' }">
        <div class="panel-header">
          <span class="panel-title">{{ activePanelConfig?.label }}</span>
          <div class="panel-ops">
            <el-button size="small" text @click="panelRefreshKey++">
              <el-icon><Refresh /></el-icon>刷新
            </el-button>
            <el-button size="small" text @click="closePanel()">
              <el-icon><Close /></el-icon>
            </el-button>
          </div>
        </div>
        <div class="panel-body">
          <component :is="activeView" :key="panelRefreshKey" />
        </div>
      </aside>
    </Transition>
  </div>
</template>

<script setup>
import { computed, markRaw, onMounted, onUnmounted, provide, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import {
  Bell, BellFilled, Close, Loading, Refresh
} from '@element-plus/icons-vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from './api'
import SetupView from './views/SetupView.vue'
import TodayView from './views/TodayView.vue'
import MoneyView from './views/MoneyView.vue'
import CalendarView from './views/CalendarView.vue'
import ProjectsView from './views/ProjectsView.vue'
import NotesView from './views/NotesView.vue'
import IdeasView from './views/IdeasView.vue'
import FloatingButton from './components/FloatingButton.vue'
import TodayTaskList from './components/TodayTaskList.vue'
import MoneyExpenseList from './components/MoneyExpenseList.vue'
import CalendarDayList from './components/CalendarDayList.vue'
import ProjectList from './components/ProjectList.vue'
import NoteList from './components/NoteList.vue'
import IdeaList from './components/IdeaList.vue'

const loading = ref(true)
const configured = ref(false)
const status = ref({ last_sync: null, last_error: null, pending: 0, remind_enabled: false })
const syncing = ref(false)
const pulling = ref(false)
const resetting = ref(false)
const activePanel = ref(null)
const panelRefreshKey = ref(0)
const timing = ref(false)
let pollTimer = null
let timingTimer = null

const todayStr = computed(() => dayjs().format('M月D日 dddd'))

// 视图映射：浮动按钮 key → 对应视图组件（面板内渲染）
const viewMap = {
  today: TodayView,
  money: MoneyView,
  calendar: CalendarView,
  projects: ProjectsView,
  notes: NotesView,
  ideas: IdeasView,
}
const subAction = ref(null)
provide('subAction', subAction)

function subActionHandler(key, action) {
  // 精简列表能直接处理的动作不弹大面板：
  //   今日任务 - 添加/排序/完成/计时（「报表」走面板）
  //   金钱     - 记一笔（「统计」走面板）
  //   项目     - 看板/甘特（dock 内切换视图）
  const dockOnly =
    (key === 'today' && action !== 'report') ||
    (key === 'money' && action === 'add') ||
    (key === 'projects' && (action === 'board' || action === 'gantt'))
  if (!dockOnly) {
    const fb = floatButtons.value.find(b => b.key === key)
    if (fb) openPanel(fb)
  }
  setTimeout(() => { subAction.value = { key, action } }, 50)
}

const floatButtons = computed(() => [
  { key: 'today', icon: '☑', label: '今日任务', color: '#409eff', x: 130, y: 150,
    subs: [
      { icon: '＋', label: '添加', handler: () => subActionHandler('today', 'add') },
      { icon: '▲', label: '排序', handler: () => subActionHandler('today', 'sort') },
      { icon: '✓', label: '完成', handler: () => subActionHandler('today', 'done') },
      { icon: '▤', label: '报表', handler: () => subActionHandler('today', 'report') },
      timing.value
        ? { icon: '⏹', label: '结束计时', handler: () => subActionHandler('today', 'stop-timing') }
        : { icon: '▶', label: '开始计时', handler: () => subActionHandler('today', 'timing') },
    ] },
  { key: 'money', icon: '💰', label: '金钱', color: '#67c23a', x: 130, y: 245,
    subs: [
      { icon: '＋', label: '记一笔', handler: () => subActionHandler('money', 'add') },
      { icon: '☷', label: '统计', handler: () => subActionHandler('money', 'summary') },
    ] },
  { key: 'calendar', icon: '📅', label: '日历', color: '#909399', x: 130, y: 340,
    subs: [] },
  { key: 'projects', icon: '📊', label: '项目', color: '#e6a23c', x: 130, y: 435,
    subs: [
      { icon: '◫', label: '看板', handler: () => subActionHandler('projects', 'board') },
      { icon: '▤', label: '甘特', handler: () => subActionHandler('projects', 'gantt') },
    ] },
  { key: 'notes', icon: '📚', label: '知识库', color: '#13c2c2', x: 130, y: 530,
    subs: [] },
  { key: 'ideas', icon: '💡', label: '好想法', color: '#faad14', x: 130, y: 625,
    subs: [] },
])
const activePanelConfig = ref(null)
const activeView = ref(null)

// ---- 精简列表跟随按钮（今日任务/金钱）----
const dockPos = reactive({
  today: { x: 130, y: 150 },
  money: { x: 130, y: 245 },
  calendar: { x: 130, y: 340 },
  projects: { x: 130, y: 435 },
  notes: { x: 130, y: 530 },
  ideas: { x: 130, y: 625 },
})
const dockOpen = reactive({
  today: false, money: false, calendar: false, projects: false, notes: false, ideas: false,
})
function onFloatMove(fb, p) {
  if (fb.key in dockPos) {
    dockPos[fb.key].x = p.x
    dockPos[fb.key].y = p.y
  }
}
function onFloatToggle(fb, opened) {
  // 展开/收起子按钮时，对应精简列表同步显示/隐藏
  if (fb.key in dockOpen) dockOpen[fb.key] = opened
}
function makeDockStyle(pos) {
  // 面板顶边与按钮顶对齐，并夹在可视区域内
  const top = Math.max(60, Math.min(pos.y - 28, window.innerHeight - 440))
  return { left: pos.x + 90 + 'px', top: top + 'px' }
}
const todayDockStyle = computed(() => makeDockStyle(dockPos.today))
const moneyDockStyle = computed(() => makeDockStyle(dockPos.money))
const calendarDockStyle = computed(() => makeDockStyle(dockPos.calendar))
const projectsDockStyle = computed(() => makeDockStyle(dockPos.projects))
const notesDockStyle = computed(() => makeDockStyle(dockPos.notes))
const ideasDockStyle = computed(() => makeDockStyle(dockPos.ideas))

function openPanel(fb) {
  activePanel.value = fb.key
  activePanelConfig.value = fb
  activeView.value = markRaw(viewMap[fb.key] || TodayView)
}
// 精简列表组件可通过 inject('openPanel')('xxx') 打开对应完整面板
provide('openPanel', (key) => {
  const fb = floatButtons.value.find(b => b.key === key)
  if (fb) openPanel(fb)
})

function onFloatOpen(fb) {
  // 有子按钮时，主按钮点击只展开/收起子按钮（由组件自己处理），不弹面板；
  // 有精简列表的按钮点击切换列表（完整面板从列表里进）；
  // 其余无子按钮的点击直接弹面板
  if (fb.subs.length === 0 && !(fb.key in dockOpen)) openPanel(fb)
}

function onFloatSub(fb, sub) {
  // 子按钮动作：打开对应面板（带 handler 的子按钮走 subActionHandler，这里是兜底）
  openPanel(fb)
}

function closePanel() {
  activePanel.value = null
  activePanelConfig.value = null
  activeView.value = null
}

// ---------- 到点提醒（后端系统通知，页面关了也有效） ----------
// 开关状态以服务端 config 为准（status.remind_enabled），不再用浏览器 Notification
const remindOn = computed(() => !!status.value.remind_enabled)
async function toggleRemind() {
  const target = !remindOn.value
  try {
    await api.toggleReminders(target)
    status.value = { ...status.value, remind_enabled: target }
    ElMessage.success(target ? '到点提醒已开启（系统通知）' : '到点提醒已关闭')
  } catch (e) { ElMessage.error(`设置失败：${e.message}`) }
}

// ---------- 全局搜索 ----------
const searchPick = ref(null)
const searching = ref(false)
const searchHits = ref([])
// 搜索跳转目标（目前只有知识库文档深链）；NotesView 挂载时消费并清空
const searchTarget = ref(null)
provide('searchTarget', searchTarget)

const KIND_META = {
  note: { label: '知识库', panel: 'notes' },
  task: { label: '任务', panel: 'today' },
  idea: { label: '想法', panel: 'ideas' },
  event: { label: '事件', panel: 'calendar' },
  expense: { label: '消费', panel: 'money' },
  project: { label: '项目', panel: 'projects' },
}
const searchGroups = computed(() =>
  Object.entries(KIND_META)
    .map(([kind, meta]) => ({ kind, label: meta.label, items: searchHits.value.filter(h => h.kind === kind) }))
    .filter(g => g.items.length > 0)
)

// 防竞态：只采纳最后一次查询的结果
let searchSeq = 0
async function doSearch(q) {
  const kw = (q || '').trim()
  if (!kw) { searchHits.value = []; return }
  const seq = ++searchSeq
  searching.value = true
  try {
    const hits = await api.search(kw)
    if (seq === searchSeq) searchHits.value = hits
  } catch { /* 搜索失败静默，不打断输入 */ }
  finally { if (seq === searchSeq) searching.value = false }
}

function onSearchPick(val) {
  if (!val) return
  const idx = val.indexOf(':')
  const kind = val.slice(0, idx)
  const id = val.slice(idx + 1)
  const meta = KIND_META[kind]
  searchPick.value = null
  searchHits.value = []
  if (!meta) return
  if (kind === 'note') searchTarget.value = { kind: 'note', id }
  const fb = floatButtons.value.find(b => b.key === meta.panel)
  if (fb) openPanel(fb)
  panelRefreshKey.value++ // 强制重挂载，让目标视图消费 searchTarget
}

// ---------- AI 快速录入 ----------
const qeDialog = ref(false)
const qeText = ref('')
const qeParsing = ref(false)
const qeSaving = ref(false)
const qeDraft = ref(null)

const QE_TYPE_LABELS = { expense: '💰 消费', event: '📅 日程事件', idea: '💡 想法', task: '☑ 任务' }
const qeTypeLabel = computed(() => (qeDraft.value ? QE_TYPE_LABELS[qeDraft.value.type] || qeDraft.value.type : ''))
const qeFields = computed(() => {
  const d = qeDraft.value
  if (!d) return []
  const dt = (ts) => dayjs.unix(ts).format('MM-DD HH:mm')
  switch (d.type) {
    case 'expense':
      return [{ k: '事项', v: d.item }, { k: '金额', v: `¥${d.amount}` }, { k: '分类', v: d.category }, { k: '时间', v: dt(d.ts) }]
    case 'event':
      return [
        { k: '内容', v: d.content }, { k: '标签', v: d.tag },
        { k: '开始', v: dt(d.start_ts) }, { k: '结束', v: d.end_ts ? dt(d.end_ts) : '（默认 1 小时）' },
        { k: '提醒', v: d.remind ? '🔔 到点系统通知' : '无' },
      ]
    case 'idea':
      return [{ k: '内容', v: d.content }, { k: '标签', v: d.tag }]
    case 'task':
      return [{ k: '标题', v: d.title }, { k: '日期', v: d.date }, { k: '优先级', v: d.priority }]
    default:
      return []
  }
})

function openQuickEntry() {
  qeText.value = ''
  qeDraft.value = null
  qeDialog.value = true
}

// Ctrl/Cmd+K 全局唤起快速录入
function onGlobalKey(e) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
    e.preventDefault()
    if (configured.value && !qeDialog.value) openQuickEntry()
  }
}

// ---------- 数据导出 ----------
async function downloadExport(kind) {
  const url = kind === 'json' ? '/api/export' : `/api/export/csv?entity=${kind}`
  try {
    const res = await fetch(url)
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    const blob = await res.blob()
    const a = document.createElement('a')
    const stamp = dayjs().format('YYYYMMDD-HHmm')
    a.href = URL.createObjectURL(blob)
    a.download = kind === 'json' ? `latte-backup-${stamp}.json` : `latte-${kind}-${stamp}.csv`
    a.click()
    URL.revokeObjectURL(a.href)
    ElMessage.success('导出成功')
  } catch (e) {
    ElMessage.error(`导出失败：${e.message}`)
  }
}

async function qeParse() {
  const text = qeText.value.trim()
  if (!text) return
  qeParsing.value = true
  try {
    qeDraft.value = await api.quickEntry(text)
  } catch (e) {
    qeDraft.value = null
    ElMessage.error(e.status === 502 ? `${e.message}（请检查 latte-model-proxy 是否启动）` : e.message)
  } finally {
    qeParsing.value = false
  }
}

async function qeConfirm() {
  const d = qeDraft.value
  if (!d) return
  qeSaving.value = true
  try {
    if (d.type === 'expense') {
      await api.createExpense({ item: d.item, amount: d.amount, category: d.category, ts: d.ts })
    } else if (d.type === 'event') {
      // 结束时间缺省按 1 小时（后端不接受无结束时间的补录事件）
      await api.createEvent({ start_ts: d.start_ts, end_ts: d.end_ts ?? d.start_ts + 3600, content: d.content, tag: d.tag, remind: d.remind })
    } else if (d.type === 'idea') {
      await api.createIdea({ content: d.content, tag: d.tag })
    } else if (d.type === 'task') {
      await api.createTask({ title: d.title, date: d.date, priority: d.priority })
    }
    ElMessage.success(`${QE_TYPE_LABELS[d.type] || '记录'}已创建`)
    qeDialog.value = false
    panelRefreshKey.value++ // 触发各列表刷新
    await fetchStatus()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    qeSaving.value = false
  }
}

async function fetchStatus() {
  try {
    const s = await api.getStatus()
    status.value = s; configured.value = !!s.configured
  } catch (e) { ElMessage.error(`获取状态失败：${e.message}`) }
  finally { loading.value = false }
}

// 计时状态：驱动「今日任务」按钮的计时动画
async function checkTiming() {
  if (!configured.value) return
  try {
    timing.value = !!(await api.getOngoingEvent())
  } catch { /* 查询失败保持原状态 */ }
}
async function doSync() {
  syncing.value = true
  try {
    const r = await api.sync()
    ElMessage.success(`同步完成，待同步 ${r.pending} 条`); await fetchStatus()
  } catch (e) { ElMessage.error(`同步失败：${e.message}`); await fetchStatus() }
  finally { syncing.value = false }
}

// 从 Notion 全量拉取覆盖本地：远端为事实来源，本地多余行会被删除
async function doPull() {
  try {
    await ElMessageBox.confirm(
      '将从 Notion 远端重新拉取全部数据并覆盖本地（本地多余的数据会被删除）。已同步行的本地专属字段（如提醒、番茄钟数）会保留。是否继续？',
      '拉取并覆盖本地',
      { confirmButtonText: '拉取', cancelButtonText: '取消', type: 'warning' }
    )
  } catch { return } // 用户取消
  pulling.value = true
  try {
    const r = await api.pullFromNotion()
    const c = r.counts || {}
    ElMessage.success(`拉取完成：事件 ${c.events ?? 0} · 消费 ${c.expenses ?? 0} · 项目 ${c.projects ?? 0} · 想法 ${c.ideas ?? 0} · 任务 ${c.tasks ?? 0} · 知识库 ${c.notes ?? 0}`)
    panelRefreshKey.value++ // 触发各列表刷新
    await fetchStatus()
  } catch (e) { ElMessage.error(`拉取失败：${e.message}`); await fetchStatus() }
  finally { pulling.value = false }
}

// 清空本地全部数据后重新从 Notion 拉取：配置（token/页面）保留，
// 但未同步的本地修改与仅本地保存的字段（提醒、番茄钟数等）会丢失
async function doReset() {
  try {
    await ElMessageBox.confirm(
      '将删除本地全部数据（事件、消费、项目、知识库、想法、任务），然后重新从 Notion 拉取。Notion 配置（token/页面）会保留；但未同步到 Notion 的本地修改、以及仅保存在本地的字段（事件提醒、番茄钟数、任务备注等）将永久丢失。是否继续？',
      '清空本地并重拉',
      { confirmButtonText: '清空并重拉', cancelButtonText: '取消', type: 'warning' }
    )
  } catch { return } // 用户取消
  resetting.value = true
  try {
    await api.resetLocalData()
    const r = await api.pullFromNotion()
    const c = r.counts || {}
    ElMessage.success(`已清空并重新拉取：事件 ${c.events ?? 0} · 消费 ${c.expenses ?? 0} · 项目 ${c.projects ?? 0} · 想法 ${c.ideas ?? 0} · 任务 ${c.tasks ?? 0} · 知识库 ${c.notes ?? 0}`)
    panelRefreshKey.value++ // 触发各列表刷新
    await fetchStatus()
  } catch (e) { ElMessage.error(`重置失败：${e.message}`); await fetchStatus() }
  finally { resetting.value = false }
}
function onSetupDone() { configured.value = true; fetchStatus() }

onMounted(() => {
  fetchStatus()
  pollTimer = setInterval(() => { if (configured.value) fetchStatus() }, 30000)
  checkTiming()
  timingTimer = setInterval(checkTiming, 5000)
  window.addEventListener('keydown', onGlobalKey)
})
onUnmounted(() => {
  clearInterval(pollTimer); clearInterval(timingTimer)
  window.removeEventListener('keydown', onGlobalKey)
})
</script>

<style scoped>
.boot { display: flex; justify-content: center; align-items: center; height: 100vh; }
.app-root { min-height: 100vh; }

.topbar {
  position: fixed; top: 0; left: 0; right: 0; z-index: 100;
  display: flex; align-items: center; gap: 16px;
  height: 48px; padding: 0 20px;
  background: rgba(255,255,255,0.85);
  backdrop-filter: blur(12px);
  border-bottom: 1px solid #e4e7ed;
}
.brand { font-weight: 700; font-size: 16px; white-space: nowrap; color: #409eff; }
.top-date { font-size: 13px; color: #909399; flex: 1; }
.top-right { display: flex; align-items: center; gap: 10px; }
.top-search { width: 200px; }
.kbd-hint { font-size: 10px; color: #b0b6bf; margin-left: 4px; }
.sync-info { font-size: 12px; color: #909399; }
.sync-error { color: #f56c6c; }
.pending-badge { margin-left: 4px; }

.main-area {
  height: calc(100vh - 48px);
  display: flex;
  justify-content: center;
  align-items: center;
}
.empty-bg { text-align: center; color: #c0c0c0; user-select: none; }
/* 今日任务列表：固定在「今日任务」按钮右侧，位置由 dockStyle 跟随按钮 */
.today-dock {
  position: fixed;
  z-index: 140;
  width: 320px;
  max-height: calc(100vh - 80px);
  overflow-y: auto;
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0,0,0,0.08);
  padding: 12px 16px;
}
/* 知识库编辑器式 dock 更宽 */
.today-dock.dock-wide {
  width: 560px;
}
/* 知识库 dock 更宽更高，便于文档阅读 */
.today-dock.dock-notes {
  width: min(860px, 92vw);
}
.empty-icon { font-size: 64px; margin-bottom: 12px; opacity: 0.5; }
.empty-hint { font-size: 14px; }

/* 面板覆盖层 */
.panel-overlay {
  position: fixed; inset: 0; z-index: 150; background: rgba(0,0,0,0.2);
}
.panel-fade-enter-active, .panel-fade-leave-active { transition: opacity 0.2s; }
.panel-fade-enter-from, .panel-fade-leave-to { opacity: 0; }

/* 右侧滑入面板 */
.side-panel {
  position: fixed; top: 0; right: 0; bottom: 0; width: min(640px, 92vw); z-index: 160;
  background: #fff; border-left: 1px solid #e4e7ed;
  box-shadow: -4px 0 20px rgba(0,0,0,0.1);
  display: flex; flex-direction: column; overflow: hidden;
}
/* 知识库完整视图需要更宽以容纳目录树+内容 */
.side-panel.side-panel-wide {
  width: min(1080px, 95vw);
}
.panel-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 16px; border-bottom: 1px solid #e4e7ed; flex-shrink: 0;
}
.panel-title { font-weight: 600; font-size: 15px; }
.panel-ops { display: flex; align-items: center; }
.panel-body { flex: 1; overflow-y: auto; padding: 12px 16px; }
.panel-slide-enter-active, .panel-slide-leave-active { transition: transform 0.25s ease; }
.panel-slide-enter-from, .panel-slide-leave-to { transform: translateX(100%); }
</style>

<!-- 搜索下拉内容 teleport 到 body，scoped 样式命中不到，单独非 scoped 块 -->
<style>
.srch-opt { display: flex; flex-direction: column; line-height: 1.4; padding: 2px 0; }
.srch-title { font-size: 13px; color: #303133; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 320px; }
.srch-snippet { font-size: 11px; color: #909399; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 320px; }
/* 快速录入预览（el-dialog 同样 teleport 到 body） */
.qe-preview { margin-top: 14px; border: 1px solid #e4e7ed; border-radius: 8px; padding: 10px 14px; background: #f8f9fb; }
.qe-type { font-size: 14px; font-weight: 600; margin-bottom: 8px; }
.qe-field { display: flex; gap: 12px; font-size: 13px; padding: 3px 0; }
.qe-k { flex: 0 0 40px; color: #909399; }
</style>
