<template>
  <div class="popup-root">
    <header class="popup-header">
      <span class="popup-title">{{ currentLabel }}</span>
      <div class="popup-ops">
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
import TodayView from './views/TodayView.vue'
import MoneyView from './views/MoneyView.vue'
import CalendarView from './views/CalendarView.vue'
import ProjectsView from './views/ProjectsView.vue'
import NotesView from './views/NotesView.vue'

const viewMap = {
  today: TodayView,
  money: MoneyView,
  calendar: CalendarView,
  projects: ProjectsView,
  notes: NotesView,
}
const LABELS = {
  today: '今日任务',
  money: '金钱',
  calendar: '日历',
  projects: '项目',
  notes: '知识库',
}

const currentKey = ref('')
const currentView = shallowRef(null)
const currentLabel = computed(() => LABELS[currentKey.value] || '')

// 各视图会 inject('subAction')（主窗口浮动按钮时代的快捷动作通道），这里提供空实现避免告警
const subAction = ref(null)
provide('subAction', subAction)

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
