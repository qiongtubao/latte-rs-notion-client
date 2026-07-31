<template>
  <div v-if="loading" class="boot">
    <el-icon class="is-loading" :size="32"><Loading /></el-icon>
  </div>

  <SetupView v-else-if="!configured" @done="onSetupDone" />

  <div v-else>
    <el-header class="topbar" height="auto">
      <div class="topbar-inner">
        <span class="brand">☕ Latte</span>
        <el-menu
          mode="horizontal"
          :default-active="tab"
          :ellipsis="false"
          class="tabs"
          @select="tab = $event"
        >
          <el-menu-item index="today">今日</el-menu-item>
          <el-menu-item index="time">时间碎片</el-menu-item>
          <el-menu-item index="money">金钱</el-menu-item>
          <el-menu-item index="calendar">日历</el-menu-item>
          <el-menu-item index="projects">项目</el-menu-item>
          <el-menu-item index="notes">知识库</el-menu-item>
          <el-menu-item index="ideas">好想法</el-menu-item>
        </el-menu>
        <div class="sync-area">
          <span class="sync-info">
            <template v-if="status.last_error">
              <span class="sync-error">{{ status.last_error }}</span>
            </template>
            <template v-else>
              最后同步：{{ status.last_sync || '从未' }}
            </template>
            <el-badge v-if="status.pending > 0" :value="status.pending" type="warning" class="pending-badge" />
          </span>
          <el-tooltip :content="remindOn ? '提醒已开启' : '开启到点提醒'" placement="bottom">
            <el-button
              size="small"
              circle
              :type="remindOn ? 'primary' : 'default'"
              @click="toggleRemind"
            >
              <el-icon><BellFilled v-if="remindOn" /><Bell v-else /></el-icon>
            </el-button>
          </el-tooltip>
          <el-button size="small" :loading="syncing" @click="doSync">立即同步</el-button>
        </div>
      </div>
    </el-header>

    <main class="page">
      <TodayView v-if="tab === 'today'" />
      <TimeView v-else-if="tab === 'time'" />
      <MoneyView v-else-if="tab === 'money'" />
      <CalendarView v-else-if="tab === 'calendar'" />
      <ProjectsView v-else-if="tab === 'projects'" />
      <NotesView v-else-if="tab === 'notes'" />
      <IdeasView v-else-if="tab === 'ideas'" />
    </main>
  </div>
</template>

<script setup>
import { onMounted, onUnmounted, ref } from 'vue'
import dayjs from 'dayjs'
import { Bell, BellFilled, Loading } from '@element-plus/icons-vue'
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

const loading = ref(true)
const configured = ref(false)
const status = ref({ last_sync: null, last_error: null, pending: 0 })
const tab = ref('today')
const syncing = ref(false)
let pollTimer = null

// ---------- 到点浏览器提醒 ----------

const remindOn = ref(localStorage.getItem('latte-remind') === '1')
let remindTimer = null
let lastCheck = dayjs().unix()

function loadNotified() {
  try {
    const data = JSON.parse(localStorage.getItem('latte-notified') || '{}')
    const today = dayjs().format('YYYY-MM-DD')
    return data.date === today ? new Set(data.ids) : new Set()
  } catch {
    return new Set()
  }
}

function saveNotified(set) {
  localStorage.setItem(
    'latte-notified',
    JSON.stringify({ date: dayjs().format('YYYY-MM-DD'), ids: [...set] })
  )
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
        new Notification('Latte 提醒', {
          body: `该开始了：${ev.content || '(无内容)'}（${ev.tag || '未分类'}）`,
        })
        notified.add(ev.id)
        changed = true
      }
    }
    if (changed) saveNotified(notified)
  } catch {
    // 提醒轮询失败静默处理，避免刷屏
  } finally {
    lastCheck = now
  }
}

async function toggleRemind() {
  if (remindOn.value) {
    remindOn.value = false
    localStorage.setItem('latte-remind', '0')
    return
  }
  if (!('Notification' in window)) {
    ElMessage.warning('当前浏览器不支持通知')
    return
  }
  const perm = await Notification.requestPermission()
  if (perm === 'granted') {
    remindOn.value = true
    localStorage.setItem('latte-remind', '1')
    lastCheck = dayjs().unix()
    ElMessage.success('到点提醒已开启')
  } else {
    ElMessage.warning('通知权限被拒绝，请到浏览器设置中允许本站通知')
  }
}

async function fetchStatus() {
  try {
    const s = await api.getStatus()
    status.value = s
    configured.value = !!s.configured
  } catch (e) {
    ElMessage.error(`获取状态失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

async function doSync() {
  syncing.value = true
  try {
    const r = await api.sync()
    ElMessage.success(`同步完成，待同步 ${r.pending} 条`)
    await fetchStatus()
  } catch (e) {
    ElMessage.error(`同步失败：${e.message}`)
    await fetchStatus()
  } finally {
    syncing.value = false
  }
}

function onSetupDone() {
  configured.value = true
  fetchStatus()
}

onMounted(() => {
  fetchStatus()
  pollTimer = setInterval(() => {
    if (configured.value) fetchStatus()
  }, 30000)
  remindTimer = setInterval(checkReminders, 30000)
})

onUnmounted(() => {
  clearInterval(pollTimer)
  clearInterval(remindTimer)
})
</script>

<style scoped>
.boot {
  display: flex;
  justify-content: center;
  align-items: center;
  height: 100vh;
}

.topbar {
  background: #fff;
  border-bottom: 1px solid #e4e7ed;
  padding: 0;
}

.topbar-inner {
  max-width: 1080px;
  margin: 0 auto;
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 0 16px;
}

.brand {
  font-weight: 700;
  font-size: 18px;
  white-space: nowrap;
}

.tabs {
  flex: 1;
  border-bottom: none;
}

.sync-area {
  display: flex;
  align-items: center;
  gap: 10px;
  white-space: nowrap;
}

.sync-info {
  font-size: 12px;
  color: #909399;
}

.sync-error {
  color: #f56c6c;
}

.pending-badge {
  margin-left: 6px;
}
</style>
