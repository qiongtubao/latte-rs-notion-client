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
    <aside v-if="todayDockOpen" class="today-dock" :style="dockStyle">
      <TodayTaskList />
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
import TimeView from './views/TimeView.vue'
import MoneyView from './views/MoneyView.vue'
import CalendarView from './views/CalendarView.vue'
import ProjectsView from './views/ProjectsView.vue'
import NotesView from './views/NotesView.vue'
import IdeasView from './views/IdeasView.vue'
import FloatingButton from './components/FloatingButton.vue'
import TodayTaskList from './components/TodayTaskList.vue'

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
  time: TimeView,
  money: MoneyView,
  calendar: CalendarView,
  projects: ProjectsView,
  notes: NotesView,
  ideas: IdeasView,
}
const subAction = ref(null)
provide('subAction', subAction)

function subActionHandler(key, action) {
  // 今日任务的子按钮动作直接作用在按钮旁的精简列表上，不再弹大面板；
  // 其他功能的子按钮仍然打开对应面板
  if (key !== 'today') {
    const fb = floatButtons.find(b => b.key === key)
    if (fb) openPanel(fb)
  }
  setTimeout(() => { subAction.value = { key, action } }, 50)
}

const floatButtons = [
  { key: 'today', icon: '☑', label: '今日任务', color: '#409eff', x: 130, y: 150,
    subs: [
      { icon: '＋', label: '添加', handler: () => subActionHandler('today', 'add') },
      { icon: '▲', label: '排序', handler: () => subActionHandler('today', 'sort') },
      { icon: '✓', label: '完成', handler: () => subActionHandler('today', 'done') },
    ] },
  { key: 'time', icon: '⏱', label: '时间碎片', color: '#e6a23c', x: 130, y: 245,
    subs: [
      { icon: '▶', label: '开始计时', handler: () => subActionHandler('time', 'start') },
      { icon: '▤', label: '报表', handler: () => subActionHandler('time', 'report') },
    ] },
  { key: 'money', icon: '💰', label: '金钱', color: '#67c23a', x: 130, y: 340,
    subs: [
      { icon: '＋', label: '记一笔', handler: () => subActionHandler('money', 'add') },
      { icon: '☷', label: '统计', handler: () => subActionHandler('money', 'summary') },
    ] },
  { key: 'calendar', icon: '📅', label: '日历', color: '#909399', x: 130, y: 435,
    subs: [] },
  { key: 'projects', icon: '📊', label: '项目', color: '#e6a23c', x: 130, y: 530,
    subs: [
      { icon: '◫', label: '看板', handler: () => subActionHandler('projects', 'board') },
      { icon: '▤', label: '甘特', handler: () => subActionHandler('projects', 'gantt') },
    ] },
  { key: 'notes', icon: '📚', label: '知识库', color: '#13c2c2', x: 130, y: 625,
    subs: [] },
  { key: 'ideas', icon: '💡', label: '好想法', color: '#faad14', x: 130, y: 720,
    subs: [] },
]
const activePanelConfig = ref(null)
const activeView = ref(null)

// ---- 今日任务列表跟随「今日任务」按钮 ----
const todayPos = reactive({ x: 130, y: 150 })
const todayDockOpen = ref(false)
function onFloatMove(fb, p) {
  if (fb.key === 'today') {
    todayPos.x = p.x
    todayPos.y = p.y
  }
}
function onFloatToggle(fb, opened) {
  // 今日任务：展开/收起子按钮时，右侧任务列表同步显示/隐藏
  if (fb.key === 'today') todayDockOpen.value = opened
}
const dockStyle = computed(() => {
  // 面板顶边与按钮顶对齐，并夹在可视区域内
  const top = Math.max(60, Math.min(todayPos.y - 28, window.innerHeight - 440))
  return { left: todayPos.x + 90 + 'px', top: top + 'px' }
})

function openPanel(fb) {
  activePanel.value = fb.key
  activePanelConfig.value = fb
  activeView.value = markRaw(viewMap[fb.key] || TodayView)
}

function onFloatOpen(fb) {
  // 有子按钮时，主按钮点击只展开/收起子按钮（由组件自己处理），不弹面板；
  // 无子按钮（如日历）点击直接弹面板
  if (fb.subs.length === 0) openPanel(fb)
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
