<template>
  <div
    class="ball"
    :class="{ opaque }"
    :style="{ background: meta.color }"
    :title="meta.label"
    @mousedown="onDown"
  >
    <span class="ball-icon">{{ meta.icon }}</span>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { invoke } from '@tauri-apps/api/core'

const props = defineProps({
  ballKey: { type: String, required: true },
})

const META = {
  today: { icon: '☑', label: '今日任务', color: '#409eff' },
  money: { icon: '💰', label: '金钱', color: '#67c23a' },
  calendar: { icon: '📅', label: '日历', color: '#909399' },
  projects: { icon: '📊', label: '项目', color: '#9b59b6' },
}
const meta = computed(() => META[props.ballKey] || { icon: '☕', label: props.ballKey, color: '#409eff' })

// 不透明模式（Linux X11 无合成器时透明失效，由壳通过 ?opaque=1 下发）：
// 窗口整窗不透明，body 底色刷成球色，圆角窗外不露白
const opaque = new URLSearchParams(location.search).has('opaque')
if (opaque) {
  document.body.style.setProperty('background', meta.value.color, 'important')
}

const win = getCurrentWindow()
const DRAG_THRESHOLD = 4 // px，超过才算拖拽，避免手抖吞掉点击

let downX = 0
let downY = 0
let dragging = false

function onDown(e) {
  if (e.button !== 0) return
  downX = e.clientX
  downY = e.clientY
  dragging = false
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('mouseup', onUp)
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
}

async function onMove(e) {
  if (dragging) return
  if (Math.hypot(e.clientX - downX, e.clientY - downY) > DRAG_THRESHOLD) {
    dragging = true
    // 交给系统原生拖拽；移动结束后的位置由后端 Moved 事件持久化
    try { await win.startDragging() } catch { /* 忽略拖拽失败 */ }
  }
}

function onUp() {
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('mouseup', onUp)
  if (!dragging) {
    // 未发生拖拽：视为点击，切换快捷操作弹窗
    invoke('toggle_popup', { key: props.ballKey }).catch(() => {})
  }
  dragging = false
}
</script>

<style>
/* 悬浮球窗口：整窗透明，只画一个圆球 */
html, body {
  background: transparent !important;
  margin: 0;
  overflow: hidden;
  height: 100%;
}
#app {
  min-height: 0 !important;
}
.ball {
  width: 48px;
  height: 48px;
  margin: 8px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: move;
  user-select: none;
  box-shadow: 0 4px 14px rgba(0, 0, 0, 0.3);
  transition: transform 0.15s, box-shadow 0.15s;
}
.ball:hover {
  transform: scale(1.08);
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.35);
}
/* 不透明模式：整窗一块圆角色块（body 已刷成球色，直接铺满窗口，图标随窗口缩放） */
.ball.opaque {
  width: 100%;
  height: 100%;
  margin: 0;
  border-radius: 12px;
}
.ball.opaque .ball-icon {
  font-size: max(20px, 28vmin);
}
.ball.opaque:hover {
  transform: none;
}
.ball-icon {
  font-size: 20px;
  color: #fff;
  line-height: 1;
  pointer-events: none;
}
</style>
