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
    <aside v-if="dockOpen.today" class="today-dock" :style="todayDockStyle">
      <TodayTaskList />
    </aside>

    <!-- 金钱列表：同上，跟随「金钱」按钮 -->
    <aside v-if="dockOpen.money" class="today-dock" :style="moneyDockStyle">
      <MoneyExpenseList />
    </aside>

    <!-- 日历列表：同上，跟随「日历」按钮 -->
    <aside v-if="dockOpen.calendar" class="today-dock" :style="calendarDockStyle">
      <CalendarDayList />
    </aside>

    <!-- 项目列表/甘特图：同上，跟随「项目」按钮 -->
    <aside v-if="dockOpen.projects" class="today-dock dock-wide" :style="projectsDockStyle">
      <ProjectList />
    </aside>

    <!-- 知识库：编辑器式布局（左目录右内容），更宽 -->
    <aside v-if="dockOpen.notes" class="today-dock dock-wide" :style="notesDockStyle">
      <NoteList />
    </aside>

    <!-- 好想法列表：同上，跟随「好想法」按钮 -->
    <aside v-if="dockOpen.ideas" class="today-dock" :style="ideasDockStyle">
      <IdeaList />
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

    <!-- 弹出面板（浮于内容之上） -->
    <Transition name="panel-fade">
      <div v-if="activePanel" class="panel-overlay" @click="closePanel()" />
    </Transition>
    <Transition name="panel-slide">
      <aside v-if="activePanel" class="side-panel">
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
import { ElMessage } from 'element-plus'
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
const status = ref({ last_sync: null, last_error: null, pending: 0 })
const syncing = ref(false)
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
  const dockOnly =
    (key === 'today' && action !== 'report') ||
    (key === 'money' && action === 'add')
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

// ---------- 到点浏览器提醒（保持不变）----------
const remindOn = ref(localStorage.getItem('latte-remind') === '1')
let remindTimer = null
let lastCheck = dayjs().unix()

function loadNotified() {
  try {
    const data = JSON.parse(localStorage.getItem('latte-notified') || '{}')
    const today = dayjs().format('YYYY-MM-DD')
    return data.date === today ? new Set(data.ids) : new Set()
  } catch { return new Set() }
}
function saveNotified(set) {
  localStorage.setItem('latte-notified',
    JSON.stringify({ date: dayjs().format('YYYY-MM-DD'), ids: [...set] }))
}
async function checkReminders() {
  if (!remindOn.value || !configured.value) return
  if (!('Notification' in window) || Notification.permission !== 'granted') return
  const today = dayjs().format('YYYY-MM-DD')
  const now = dayjs().unix()
  try {
    const events = await api.getEvents(today)
    const notified = loadNotified()
    let changed = false
    for (const ev of events) {
      if (!ev.remind || notified.has(ev.id)) continue
      if (ev.start_ts > lastCheck && ev.start_ts <= now) {
        new Notification('Latte 提醒', { body: `该开始了：${ev.content || '(无内容)'}（${ev.tag || '未分类'}）` })
        notified.add(ev.id); changed = true
      }
    }
    if (changed) saveNotified(notified)
  } catch {} finally { lastCheck = now }
}
async function toggleRemind() {
  if (remindOn.value) { remindOn.value = false; localStorage.setItem('latte-remind', '0'); return }
  if (!('Notification' in window)) { ElMessage.warning('当前浏览器不支持通知'); return }
  const perm = await Notification.requestPermission()
  if (perm === 'granted') {
    remindOn.value = true; localStorage.setItem('latte-remind', '1')
    lastCheck = dayjs().unix(); ElMessage.success('到点提醒已开启')
  } else ElMessage.warning('通知权限被拒绝')
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
function onSetupDone() { configured.value = true; fetchStatus() }

onMounted(() => {
  fetchStatus()
  pollTimer = setInterval(() => { if (configured.value) fetchStatus() }, 30000)
  remindTimer = setInterval(checkReminders, 30000)
  checkTiming()
  timingTimer = setInterval(checkTiming, 5000)
})
onUnmounted(() => {
  clearInterval(pollTimer); clearInterval(remindTimer); clearInterval(timingTimer)
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
  position: fixed; top: 0; right: 0; bottom: 0; width: 440px; z-index: 160;
  background: #fff; border-left: 1px solid #e4e7ed;
  box-shadow: -4px 0 20px rgba(0,0,0,0.1);
  display: flex; flex-direction: column; overflow: hidden;
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
