<template>
  <div class="popup-root">
    <header class="popup-header">
      <span class="popup-title">{{ currentLabel }}</span>
      <div class="popup-ops">
        <!-- 今日任务小按钮：和 Web 版悬浮子按钮动作一致 -->
        <template v-if="currentKey === 'today'">
          <el-button size="small" text title="添加任务" @click="fireSubAction('add')">＋</el-button>
          <el-button size="small" text title="切换排序" @click="fireSubAction('sort')">▲</el-button>
          <el-button size="small" text title="显示/隐藏已完成" @click="fireSubAction('done')">✓</el-button>
        </template>
        <el-button size="small" text title="打开主窗口" @click="openMain">
          <el-icon><FullScreen /></el-icon>
        </el-button>
        <el-button size="small" text title="关闭" @click="close">
          <el-icon><Close /></el-icon>
        </el-button>
      </div>
    </header>
    <main class="popup-body">
      <component :is="currentView" v-if="currentView" :key="currentKey" />
      <div v-else class="popup-empty">点击悬浮球打开对应功能</div>
    </main>
  </div>
</template>

<script setup>
import { computed, markRaw, onMounted, onUnmounted, provide, ref, shallowRef } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { Close, FullScreen } from '@element-plus/icons-vue'
import { ElMessage } from 'element-plus'
import { api } from './api'
import TodayTaskList from './components/TodayTaskList.vue'
import MoneyView from './views/MoneyView.vue'
import CalendarView from './views/CalendarView.vue'
import ProjectList from './components/ProjectList.vue'
import NoteList from './components/NoteList.vue'
import IdeaList from './components/IdeaList.vue'

const viewMap = {
  // 今日任务使用精简列表（按优先级排序），和 Web 版悬浮按钮弹出的列表一致
  today: TodayTaskList,
  money: MoneyView,
  calendar: CalendarView,
  // 项目/知识库同样使用 Web 版悬浮窗口的精简组件
  projects: ProjectList,
  notes: NoteList,
  ideas: IdeaList,
}
const LABELS = {
  today: '今日任务',
  money: '金钱',
  calendar: '日历',
  projects: '项目',
  notes: '知识库',
  ideas: '好想法',
}

const currentKey = ref('')
const currentView = shallowRef(null)
const currentLabel = computed(() => LABELS[currentKey.value] || '')

// 各视图会 inject('subAction')（主窗口浮动按钮时代的快捷动作通道），这里提供实现
const subAction = ref(null)
provide('subAction', subAction)

// 番茄钟启动：和 App.vue 提供的行为一致
provide('pomodoroStart', async (taskId, minutes) => {
  try {
    const r = await api.pomodoroTask(taskId, minutes)
    ElMessage.success(`🍅 ${r.minutes} 分钟番茄钟已启动`)
  } catch (e) {
    ElMessage.error(e.message)
  }
})

function fireSubAction(action) {
  subAction.value = { key: currentKey.value, action }
}

let unlisten = null
onMounted(async () => {
  unlisten = await listen('popup-set', (event) => {
    const key = event.payload
    currentKey.value = key
    currentView.value = markRaw(viewMap[key] || null)
  })
})
onUnmounted(() => { if (unlisten) unlisten() })

function openMain() {
  invoke('show_main').catch(() => {})
}
function close() {
  invoke('hide_popup').catch(() => {})
}
</script>

<style>
/* 弹窗窗口：不透明白底，填满整窗 */
html, body {
  background: #fff !important;
  margin: 0;
  overflow: hidden;
}
#app {
  min-height: 0 !important;
}
.popup-root {
  height: 100vh;
  display: flex;
  flex-direction: column;
  border: 1px solid #e4e7ed;
  border-radius: 10px;
  overflow: hidden;
  box-shadow: 0 8px 30px rgba(0, 0, 0, 0.18);
}
.popup-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid #e4e7ed;
  flex-shrink: 0;
}
.popup-title {
  font-weight: 600;
  font-size: 14px;
}
.popup-ops {
  display: flex;
  align-items: center;
}
.popup-body {
  flex: 1;
  overflow-y: auto;
  padding: 10px 12px;
}
.popup-empty {
  color: #909399;
  font-size: 13px;
  text-align: center;
  margin-top: 40px;
}
</style>
