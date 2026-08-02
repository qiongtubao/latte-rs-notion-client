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

    <!-- 主区域：干净背景（所有功能都从浮动按钮进入） -->
    <main class="main-area">
      <div class="empty-bg">
        <div class="empty-icon">☕</div>
        <p class="empty-hint">点击浮动按钮开始</p>
      </div>
    </main>

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
      @open="onFloatOpen(fb)"
      @sub="onFloatSub(fb, $event)"
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
import { computed, markRaw, onMounted, onUnmounted, ref } from 'vue'
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

const loading = ref(true)
const configured = ref(false)
const status = ref({ last_sync: null, last_error: null, pending: 0 })
const syncing = ref(false)
const activePanel = ref(null)
const panelRefreshKey = ref(0)
let pollTimer = null

const todayStr = computed(() => dayjs().format('M月D日 dddd'))

// ---------- 浮动按钮配置 ----------
const floatButtons = [
  { key: 'today', icon: '☑', label: '今日任务', color: '#409eff', x: 70, y: 220,
    subs: [
      { icon: '＋', label: '添加', handler: () => ElMessage.info('添加今日任务') },
      { icon: '▲', label: '排序', handler: () => ElMessage.info('按优先级排序') },
      { icon: '✓', label: '完成', handler: () => ElMessage.info('标记完成') },
    ] },
  { key: 'time', icon: '⏱', label: '时间碎片', color: '#e6a23c', x: 200, y: 220,
    subs: [
      { icon: '▶', label: '开始计时', handler: () => ElMessage.success('已开始计时') },
      { icon: '▤', label: '报表', handler: () => ElMessage.info('时间报表') },
    ] },
  { key: 'money', icon: '💰', label: '金钱', color: '#67c23a', x: 330, y: 220,
    subs: [
      { icon: '＋', label: '记一笔', handler: () => ElMessage.info('快速记账') },
      { icon: '☷', label: '统计', handler: () => ElMessage.info('消费统计') },
    ] },
  { key: 'calendar', icon: '📅', label: '日历', color: '#909399', x: 460, y: 220,
    subs: [] },
  { key: 'projects', icon: '📊', label: '项目', color: '#e6a23c', x: 70, y: 380,
    subs: [
      { icon: '◫', label: '看板', handler: () => ElMessage.info('项目看板') },
      { icon: '▤', label: '甘特', handler: () => ElMessage.info('甘特图') },
    ] },
]
const activePanelConfig = ref(null)
const activeView = ref(null)

function onFloatOpen(fb) {
  // 所有按钮都弹出对应面板（含今日任务）
  activePanel.value = fb.key
  activePanelConfig.value = fb
  activeView.value = markRaw(viewMap[fb.key] || TodayView)
}

function onFloatSub(fb, sub) {
  // 子按钮动作：打开对应面板
  activePanel.value = fb.key
  activePanelConfig.value = fb
  activeView.value = markRaw(viewMap[fb.key] || TodayView)
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
})
onUnmounted(() => {
  clearInterval(pollTimer); clearInterval(remindTimer)
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
