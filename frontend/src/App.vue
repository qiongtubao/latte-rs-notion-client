<template>
  <div v-if="loading" class="boot">
    <el-icon class="is-loading" :size="32"><Loading /></el-icon>
  </div>

  <SetupView v-else-if="!configured || showSetup" :closable="configured" :section="setupSection" @done="onSetupDone" />

  <div v-else :class="['app-root', { 'is-offline': !online }]">
    <!-- 离线状态条：顶栏之上，置顶 + 醒目 -->
    <div v-if="!online" class="offline-bar" role="status">
      <el-icon><Warning /></el-icon>
      <span>当前处于离线状态 — 同步/拉取/AI 功能暂不可用，本地数据正常读写。</span>
    </div>

    <!-- 极简顶栏 -->
    <header class="topbar">
      <span class="brand">☕ Latte</span>
      <span class="top-date">{{ todayStr }}</span>
      <div class="top-right">
        <el-button size="small" @click="openQuickEntry">⚡ 快录<span class="kbd-hint">Ctrl+K · Alt+Q</span></el-button>
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
          <template v-if="!online">
            <span class="sync-offline">⚠ 离线</span>
          </template>
          <template v-else-if="status.last_error">
            <span class="sync-error">{{ status.last_error }}</span>
          </template>
          <template v-else>
            同步 {{ status.last_sync ? dayjs(status.last_sync).format('HH:mm') : '从未' }}
          </template>
          <el-badge v-if="status.pending > 0" :value="status.pending" type="warning" class="pending-badge" />
        </span>
        <el-button size="small" circle :type="darkMode ? 'primary' : 'default'" :title="darkMode ? '切换到亮色' : '切换到暗色'" @click="toggleDark">
          <el-icon><Moon v-if="!darkMode" /><Sunny v-else /></el-icon>
        </el-button>
         <el-button size="small" circle :type="remindOn ? 'primary' : 'default'" @click="toggleRemind">
          <el-icon><BellFilled v-if="remindOn" /><Bell v-else /></el-icon>
        </el-button>
        <el-dropdown trigger="click" @command="onSettingsCommand">
          <el-button size="small">⚙ 设置</el-button>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="sync">同步</el-dropdown-item>
              <el-dropdown-item command="pull">拉取（远端覆盖本地）</el-dropdown-item>
              <el-dropdown-item divided command="export-json">导出全量备份（JSON）</el-dropdown-item>
              <el-dropdown-item command="export-events">导出事件（CSV）</el-dropdown-item>
              <el-dropdown-item command="export-expenses">导出消费（CSV）</el-dropdown-item>
              <el-dropdown-item command="import">导入</el-dropdown-item>
              <el-dropdown-item divided command="reset">重置（清空本地并重拉）</el-dropdown-item>
              <el-dropdown-item divided command="db-merge">合并数据库</el-dropdown-item>
              <el-dropdown-item command="db-delete">删除数据库</el-dropdown-item>
              <el-dropdown-item command="db-rebind">重新绑定数据库</el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </header>
    <!-- 番茄钟进行中：贴在顶栏下方的红色进度条 + 倒计时 -->
    <div v-if="pomodoro" class="pomobar">
      <div class="pomobar-bg" :style="{ width: pomodoroProgress + '%' }" />
      <div class="pomobar-text">
        🍅 <strong>{{ pomodoro.task_title }}</strong>
        <span class="pomobar-time">还剩 {{ pomodoroRemaining }}</span>
        <el-button size="small" text type="danger" @click="cancelPomodoro()">⏹ 停止</el-button>
      </div>
    </div>


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
        ref="qeInputRef"
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

    <!-- 浮动按钮的 AI 子按钮：按面板给出建议 + 可采纳条目 -->
    <AiAssistDialog
      :open="aiPanel !== null"
      :panel="aiPanel || 'today'"
      :title="aiPanel ? (AI_PANEL_META[aiPanel]?.title || '🤖 AI 助手') : '🤖 AI 助手'"
      :placeholder="aiPanel ? (AI_PANEL_META[aiPanel]?.placeholder || '') : ''"
      :context="aiContext"
      :prefill="aiPrefill"
      @close="aiPanel = null"
      @adopted="onAiAdopted"
      @idea-to-task="onIdeaToTask"
    />
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
import { computed, inject, markRaw, nextTick, onMounted, onUnmounted, provide, reactive, ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from './api'
import { Bell, BellFilled, Close, Loading, Moon, Refresh, Sunny, Warning } from '@element-plus/icons-vue'
import dayjs from 'dayjs'
import TodayView from './views/TodayView.vue'
import MoneyView from './views/MoneyView.vue'
import CalendarView from './views/CalendarView.vue'
import ProjectsView from './views/ProjectsView.vue'
import NotesView from './views/NotesView.vue'
import IdeasView from './views/IdeasView.vue'
import DailyView from './views/DailyView.vue'
import ReportView from './views/ReportView.vue'
import SetupView from './views/SetupView.vue'
import FloatingButton from './components/FloatingButton.vue'
import TodayTaskList from './components/TodayTaskList.vue'
import MoneyExpenseList from './components/MoneyExpenseList.vue'
import CalendarDayList from './components/CalendarDayList.vue'
import ProjectList from './components/ProjectList.vue'
import NoteList from './components/NoteList.vue'
import IdeaList from './components/IdeaList.vue'
import AiAssistDialog from './components/AiAssistDialog.vue'

const loading = ref(true)
const configured = ref(false)
const showSetup = ref(false) // 已配置后从「设置」重新打开 SetupView
const setupSection = ref('') // ''=全部 / merge / delete / rebind
const status = ref({ last_sync: null, last_error: null, pending: 0, remind_enabled: false })
const syncing = ref(false)
const pulling = ref(false)
const resetting = ref(false)
const activePanel = ref(null)
const panelRefreshKey = ref(0)
const timing = ref(false)
// 网络在线状态：监听 online/offline 事件；离线时禁用同步/拉取
const online = ref(typeof navigator !== 'undefined' ? navigator.onLine : true)
function onOnline() { online.value = true }
function onOffline() { online.value = false }
let pollTimer = null
let timingTimer = null
// 番茄钟：会话进行中时 { task_id, task_title, end_ts, minutes }，否则 null
const pomodoro = ref(null)
const pomodoroRemaining = ref('') // 人类可读 mm:ss
let pomodoroTick = null
let pomodoroPoll = null
// 让任务卡（TodayTaskList）唤起番茄钟
provide('pomodoroStart', async (taskId, minutes) => {
  try {
    const r = await api.pomodoroTask(taskId, minutes)
    // 启动成功后立刻拉一次（接口已返回字段，但保险起见也再拉）
    await refreshPomodoro()
    ElMessage.success(`🍅 ${r.minutes} 分钟番茄钟已启动`)
  } catch (e) {
    ElMessage.error(e.message)
  }
})
provide('pomodoroCancel', async () => {
  try {
    await api.cancelPomodoro()
    pomodoro.value = null
    ElMessage.info('番茄钟已停止')
  } catch (e) {
    ElMessage.error(e.message)
  }
})
const viewMap = {
  today: TodayView,
  money: MoneyView,
  calendar: CalendarView,
  projects: ProjectsView,
  notes: NotesView,
  ideas: IdeasView,
  daily: DailyView,
  report: ReportView,
}
const subAction = ref(null)
provide('subAction', subAction)

function subActionHandler(key, action) {
  // 精简列表能直接处理的动作不弹大面板：
  //   今日任务 - 添加/排序/完成/计时（「报表」走面板）
  //   金钱     - 记一笔（「统计」走面板）
  //   项目     - 看板/甘特（dock 内切换视图）
  const PANEL_ACTIONS = { today: ['report'], money: ['summary'] }
  const fire = () => {
    // 每次赋新对象，保证连续点同一动作也能触发各列表的 watch
    subAction.value = { key, action }
  }
  if ((PANEL_ACTIONS[key] || []).includes(action)) {
    const fb = floatButtons.value.find((b) => b.key === key)
    if (fb && activePanel.value !== key) openPanel(fb)
    // 等面板视图挂载后再下发，否则 watch 收不到
    nextTick(fire)
  } else {
    fire()
  }
}

const AI_PANEL_META = {
  today: { title: '🤖 安排任务', placeholder: '比如：明天的重点是写周报、跑步 30 分钟、整理收件箱' },
  money: { title: '🤖 记账思路', placeholder: '比如：帮我分析本月餐饮开销、给点省钱思路、或直接说"今天午饭25、咖啡12"' },
  calendar: { title: '🤖 安排日程', placeholder: '比如：明天下午3点开会提醒我；周三晚上7点健身一小时' },
  projects: { title: '🤖 规划项目', placeholder: '比如：我想做一个"读 30 本书"项目，帮我列目标和下一步' },
  notes: { title: '🤖 写文档', placeholder: '比如：帮我写一篇 Rust 入门笔记，要点：所有权、生命周期、async' },
  ideas: { title: '🤖 捕捉灵感', placeholder: '比如：读《系统之美》时关于反馈环的思考；想做个小工具…' },
  daily: { title: '🤖 打卡建议', placeholder: '比如：想养成早起 + 每天阅读的习惯，给点坚持思路 / 推荐一些打卡项' },
}

// 打开「AI 总结」（日报/周报/月报）：report 不是浮动按钮，直接经 openPanel 打开面板
function openReport() {
  const fb = { key: 'report', label: 'AI 总结', subs: [] }
  openPanel(fb)
}
const aiPanel = ref(null) // null = 关闭；否则为 panel key
function openAiAssist(key) {
  // 面板打开时收起悬浮子按钮，避免视觉重叠
  const fb = floatButtons.value.find(b => b.key === key)
  if (fb && dockOpen[key]) dockOpen[key] = false
  aiPanel.value = key
}
const aiContext = computed(() => '')
// 想法 → 任务：把想法正文作为初始文本注入到「今日任务」AI 对话框
const aiPrefill = ref('')
function onAiAdopted() {
  // 任何面板的 AI 采纳后都触发全量刷新，与现有「拉取/重置」行为一致
  panelRefreshKey.value++
}
function onIdeaToTask(ideaContent) {
  // 关闭当前 ideas 对话框（aiPanel 已经在子按钮触发 idea 落库时清空对应卡片，但保持打开），
  // 切到 today，预填想法内容
  aiPrefill.value = `基于这个想法拆成可执行任务：${ideaContent}`
  aiPanel.value = 'today'
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
      { icon: '🤖', label: 'AI 安排', handler: () => openAiAssist('today') },
      { icon: '📝', label: 'AI 总结', handler: () => openReport() },
    ] },
  { key: 'money', icon: '💰', label: '金钱', color: '#67c23a', x: 130, y: 245,
    subs: [
      { icon: '＋', label: '记一笔', handler: () => subActionHandler('money', 'add') },
      { icon: '☷', label: '统计', handler: () => subActionHandler('money', 'summary') },
      { icon: '🤖', label: 'AI 思路', handler: () => openAiAssist('money') },
    ] },
  { key: 'calendar', icon: '📅', label: '日历', color: '#909399', x: 130, y: 340,
    subs: [
      { icon: '🤖', label: 'AI 安排', handler: () => openAiAssist('calendar') },
    ] },
  { key: 'projects', icon: '📊', label: '项目', color: '#e6a23c', x: 130, y: 435,
    subs: [
      { icon: '◫', label: '看板', handler: () => subActionHandler('projects', 'board') },
      { icon: '▤', label: '甘特', handler: () => subActionHandler('projects', 'gantt') },
      { icon: '🤖', label: 'AI 规划', handler: () => openAiAssist('projects') },
    ] },
  { key: 'notes', icon: '📚', label: '知识库', color: '#13c2c2', x: 130, y: 530,
    subs: [
      { icon: '🤖', label: 'AI 写文档', handler: () => openAiAssist('notes') },
    ] },
  { key: 'ideas', icon: '💡', label: '好想法', color: '#faad14', x: 130, y: 625,
    subs: [
      { icon: '🤖', label: 'AI 捕捉', handler: () => openAiAssist('ideas') },
    ] },
  { key: 'daily', icon: '✅', label: '打卡', color: '#fa8c16', x: 130, y: 720,
    subs: [
      { icon: '🤖', label: 'AI 建议', handler: () => openAiAssist('daily') },
    ] },
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
function makeDockStyle(pos, width = 320) {
  // 面板顶边与按钮顶对齐，并夹在可视区域内
  const top = Math.max(60, Math.min(pos.y - 28, window.innerHeight - 440))
  // 宽面板（项目/知识库）左移，防止超出右边缘
  const left = Math.max(8, Math.min(pos.x + 90, window.innerWidth - width - 16))
  return { left: left + 'px', top: top + 'px' }
}
const todayDockStyle = computed(() => makeDockStyle(dockPos.today))
const moneyDockStyle = computed(() => makeDockStyle(dockPos.money))
const calendarDockStyle = computed(() => makeDockStyle(dockPos.calendar))
const projectsDockStyle = computed(() => makeDockStyle(dockPos.projects, 640))
const notesDockStyle = computed(() => makeDockStyle(dockPos.notes, 860))
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

// ---------- 暗色主题：localStorage 持久化 + html.dark 类切换 ----------
const THEME_KEY = 'latte-theme'
function readInitialTheme() {
  try { return localStorage.getItem(THEME_KEY) === 'dark' } catch { return false }
}
const darkMode = ref(readInitialTheme())
function applyTheme(v) {
  if (typeof document === 'undefined') return
  document.documentElement.classList.toggle('dark', v)
  try { localStorage.setItem(THEME_KEY, v ? 'dark' : 'light') } catch { /* 忽略 */ }
}
function toggleDark() {
  darkMode.value = !darkMode.value
  applyTheme(darkMode.value)
}
onMounted(() => applyTheme(darkMode.value))

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
const qeInputRef = ref(null)

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
  // 弹窗渲染完成后聚焦输入框，便于全局快捷键唤出后直接打字
  nextTick(() => qeInputRef.value?.focus?.())
}

// Ctrl/Cmd+K 全局唤起快速录入
function onGlobalKey(e) {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
    e.preventDefault()
    if (configured.value && !qeDialog.value) openQuickEntry()
  }
}

// ---------- 数据导出 / 导入 ----------
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

// 导入：选择了 JSON 备份后 POST 回 /api/import（INSERT OR REPLACE，幂等）
function pickImportFile() {
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = '.json,application/json'
  input.onchange = async () => {
    const file = input.files && input.files[0]
    if (!file) return
    try {
      const text = await file.text()
      const payload = JSON.parse(text)
      const result = await api.importData(payload)
      const c = result.imported || {}
      const parts = Object.entries(c).filter(([, n]) => n > 0).map(([k, n]) => `${k} ${n}`)
      ElMessage.success(parts.length ? `导入成功：${parts.join(' · ')}` : '导入完成（无新数据）')
      panelRefreshKey.value++
      await fetchStatus()
    } catch (e) {
      ElMessage.error(`导入失败：${e.message}`)
    }
  }
  input.click()
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

// 番茄钟：5s 拉一次会话状态，1s 更新一次倒计时显示
async function refreshPomodoro() {
  if (!configured.value) return
  try {
    const s = await api.getPomodoro()
    pomodoro.value = s && s.task_id ? s : null
    updatePomodoroRemaining()
  } catch { /* 忽略：pomodoro 失败不该影响其他 */ }
}
function updatePomodoroRemaining() {
  if (!pomodoro.value) { pomodoroRemaining.value = ''; return }
  const left = pomodoro.value.end_ts - Math.floor(Date.now() / 1000)
  if (left <= 0) { pomodoroRemaining.value = '00:00'; return }
  const m = Math.floor(left / 60), s = left % 60
  pomodoroRemaining.value = `${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`
}
function startPomodoroTick() {
  stopPomodoroTick()
  pomodoroTick = setInterval(updatePomodoroRemaining, 1000)
}
function stopPomodoroTick() {
  if (pomodoroTick) { clearInterval(pomodoroTick); pomodoroTick = null }
}
watch(pomodoro, (v) => {
  if (v) startPomodoroTick()
  else stopPomodoroTick()
})
const pomodoroProgress = computed(() => {
  if (!pomodoro.value) return 0
  const total = (pomodoro.value.minutes || 25) * 60
  const left = Math.max(0, pomodoro.value.end_ts - Math.floor(Date.now() / 1000))
  return Math.min(100, Math.round((1 - left / total) * 100))
})
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
    // 先解析并校验数据库映射（远端为事实来源）：映射不可用（旧 ID 失效 / 存在多个
    // 同名库）时同步接口返回 409，这里在删除本地数据前就终止，避免本地被清空后
    // 却无法从 Notion 恢复。
    await api.pullFromNotion()
    await api.resetLocalData()
    const r = await api.pullFromNotion()
    const c = r.counts || {}
    ElMessage.success(`已清空并重新拉取：事件 ${c.events ?? 0} · 消费 ${c.expenses ?? 0} · 项目 ${c.projects ?? 0} · 想法 ${c.ideas ?? 0} · 任务 ${c.tasks ?? 0} · 知识库 ${c.notes ?? 0}`)
    panelRefreshKey.value++ // 触发各列表刷新
    await fetchStatus()
  } catch (e) { ElMessage.error(`重置失败：${e.message}`); await fetchStatus() }
  finally { resetting.value = false }
}
function onSetupDone() { configured.value = true; showSetup.value = false; fetchStatus() }

// 顶栏「设置」下拉：集中同步/拉取/重置/导入导出与数据库管理入口
function onSettingsCommand(cmd) {
  if (cmd === 'sync') doSync()
  else if (cmd === 'pull') doPull()
  else if (cmd === 'reset') doReset()
  else if (cmd === 'import') pickImportFile()
  else if (cmd === 'export-json') downloadExport('json')
  else if (cmd === 'export-events') downloadExport('events')
  else if (cmd === 'export-expenses') downloadExport('expenses')
  else if (cmd.startsWith('db-')) {
    setupSection.value = cmd.slice(3) // merge / delete / rebind
    showSetup.value = true
  }
}

// ---------- Tauri 全局快捷键（Alt+Q）→ 唤起快速录入 ----------
// 仅桌面壳注入 Tauri 时生效；浏览器/纯 Web 下静默跳过
let tauriUnlisten = null
async function setupTauriQuickEntry() {
  try {
    // 动态导入：避免 Web 版打包时硬依赖 Tauri API
    const { listen } = await import('@tauri-apps/api/event')
    tauriUnlisten = await listen('latte-global-quick-entry', () => {
      if (configured.value) openQuickEntry()
    })
  } catch {
    tauriUnlisten = null
  }
}

onMounted(() => {
  fetchStatus()
  pollTimer = setInterval(() => { if (configured.value) fetchStatus() }, 30000)
  checkTiming()
  timingTimer = setInterval(checkTiming, 5000)
  refreshPomodoro()
  pomodoroPoll = setInterval(refreshPomodoro, 5000)
  window.addEventListener('keydown', onGlobalKey)
  window.addEventListener('online', onOnline)
  window.addEventListener('offline', onOffline)
  setupTauriQuickEntry()
})
onUnmounted(() => {
  clearInterval(pollTimer); clearInterval(timingTimer)
  clearInterval(pomodoroPoll); stopPomodoroTick()
  window.removeEventListener('keydown', onGlobalKey)
  window.removeEventListener('online', onOnline)
  window.removeEventListener('offline', onOffline)
  if (tauriUnlisten) tauriUnlisten()
})
</script>

<style scoped>
/* 离线状态条：置顶于顶栏之上 */
.offline-bar {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  z-index: 200;
  height: 32px;
  background: #e6a23c;
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 500;
  box-shadow: 0 1px 3px rgba(0,0,0,0.08);
}
.offline-bar .el-icon { font-size: 16px; }
/* 离线时顶栏下移避免被灰条挡住 */
.app-root.is-offline .topbar { top: 32px; }
.app-root.is-offline .pomobar { top: 80px; }

 .kbd-hint { font-size: 10px; color: #b0b6bf; margin-left: 4px; }

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

/* 番茄钟进行中顶栏：贴顶栏下方的进度条 */
.pomobar {
  position: fixed;
  top: 48px;
  left: 0;
  right: 0;
  z-index: 99;
  height: 32px;
  background: #fff5f5;
  border-bottom: 1px solid #fbc4c4;
  overflow: hidden;
  display: flex;
  align-items: center;
}
.pomobar-bg {
  position: absolute;
  top: 0;
  left: 0;
  bottom: 0;
  background: linear-gradient(90deg, #ffe7e7, #ffb3b3);
  transition: width 1s linear;
  z-index: 0;
}
.pomobar-text {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 16px;
  font-size: 13px;
  color: #c45656;
  width: 100%;
}
.pomobar-time {
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}
.sync-error { color: #f56c6c; }
.sync-offline { color: #e6a23c; font-weight: 600; }
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
/* 项目（看板/甘特）与知识库（目录+编辑器）需要更宽 */
.today-dock.dock-wide {
  width: min(640px, calc(100vw - 32px));
}
.today-dock.dock-notes {
  width: min(860px, calc(100vw - 32px));
}
.empty-bg {
  width: min(860px, 92vw);
  padding: 60px 0;
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

<style>
/* 暗色主题：覆盖项目内建的硬编码颜色（Element Plus 自带深色） */
html.dark .topbar { background: rgba(33,33,33,0.92); border-bottom-color: #3a3a3a; }
html.dark .topbar .brand { color: #66b1ff; }
html.dark .topbar .top-date { color: #c0c4cc; }
html.dark .topbar .top-right .sync-info,
html.dark .topbar .top-right .kbd-hint { color: #909399; }
html.dark .topbar .top-right .sync-error { color: #f56c6c; }
html.dark .topbar .top-right .sync-offline { color: #e6a23c; }
html.dark .main-area { background: #1f1f1f; }
html.dark .main-area .empty-bg { color: #666; }
html.dark .today-dock { background: #2a2a2a; border-color: #3a3a3a; }
html.dark .pomobar { background: #2a1f1f; border-bottom-color: #5a2a2a; color: #f89898; }
html.dark .pomobar .pomobar-time { color: #ffb3b3; }
html.dark .offline-bar { background: #b88230; }
html.dark .side-panel { background: #2a2a2a; border-left-color: #3a3a3a; }
html.dark .panel-header { border-bottom-color: #3a3a3a; }
html.dark .panel-title { color: #e0e0e0; }
html.dark .qe-preview { background: #1f1f1f; border-color: #3a3a3a; }
html.dark .qe-k { color: #888; }
html.dark .empty-icon { color: #555; }

/* 知识库：dock 精简版（NoteList）+ 完整面板（NotesView）的文档与目录暗色 */
html.dark .nte-title,
html.dark .nte-doc-title { color: #e0e0e0; }
html.dark .nte-doc-header { border-bottom-color: #3a3a3a; }
html.dark .nte-tree { border-right-color: #3a3a3a; }
html.dark .nte-node { color: #c0c4cc; }
html.dark .nte-node:hover { background: #363636; }
html.dark .nte-node.active { background: #1d3043; color: #66b1ff; }
html.dark .ntl-empty { border-color: #4a4a4a; }
html.dark .md-body,
html.dark .markdown-body { color: #d4d4d4; }
html.dark .md-body code,
html.dark .markdown-body code { background: #3d3d3d; color: #e0e0e0; }
html.dark .md-body pre,
html.dark .markdown-body pre { background: #1f1f1f; }
html.dark .md-body pre code,
html.dark .markdown-body pre code { background: none; }
html.dark .md-body blockquote,
html.dark .markdown-body blockquote { color: #909399; border-left-color: #4a4a4a; }
html.dark .md-body th, html.dark .md-body td,
html.dark .markdown-body th, html.dark .markdown-body td { border-color: #4a4a4a; }
/* 知识库右键菜单（Teleport 到 body） */
html.dark .nte-ctxmenu { background: #2a2a2a; border-color: #3a3a3a; }
html.dark .nte-ctx-title { border-bottom-color: #3a3a3a; }
html.dark .nte-ctx-item { color: #d4d4d4; }
html.dark .nte-ctx-item:hover { background: #1d3043; color: #66b1ff; }
</style>
