<template>
  <div
    class="float-group"
    :class="{ opened }"
    :style="{ left: pos.x + 'px', top: pos.y + 'px' }"
  >
    <!-- 主按钮 -->
    <button
      class="fab-main"
      :style="{ color: propsColor }"
      :class="{ emphasized: emphasized }"
      @mousedown="onDown"
      @click="onClick"
    >
      <span>{{ icon }}</span>
      <span class="fab-tooltip">{{ label }}</span>
    </button>

    <!-- 子按钮：围绕主按钮圆心，按数量均匀分布在左侧 150°，点击主按钮后显示 -->
    <button
      v-for="(s, i) in subs"
      :key="i"
      class="fab-sub"
      :style="subPos(i)"
      :title="s.label"
      @click.stop="handleSub(s)"
    >
      {{ s.icon }}
      <span class="fab-subtip">{{ s.label }}</span>
    </button>
  </div>
</template>

<script setup>
import { computed, reactive, ref } from 'vue'

const props = defineProps({
  id: { type: String, required: true },
  icon: { type: String, required: true },
  label: { type: String, default: '' },
  color: { type: String, default: '#409eff' },
  emphasized: { type: Boolean, default: false },
  x: { type: Number, default: 60 },
  y: { type: Number, default: null },
  subs: { type: Array, default: () => [] },
})

const emit = defineEmits(['open', 'toggle', 'sub'])
const propsColor = computed(() => props.color)

// ---- 位置持久化（localStorage）----
const STORE_KEY = `latte-fab-${props.id}`
function loadPos() {
  try {
    const raw = localStorage.getItem(STORE_KEY)
    if (raw) {
      const p = JSON.parse(raw)
      if (typeof p.x === 'number' && typeof p.y === 'number') return { x: p.x, y: p.y }
    }
  } catch { /* 忽略损坏数据 */ }
  return { x: props.x, y: props.y ?? window.innerHeight - 120 }
}
const pos = reactive(loadPos())
function savePos() {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify({ x: pos.x, y: pos.y }))
  } catch { /* 忽略存储失败 */ }
}

// ---- 子按钮位置：按数量均匀分布在左侧 150° 扇形 ----
const SUB_RADIUS = 74
function subPos(i) {
  const n = props.subs.length
  if (n <= 1) {
    return { left: '65px', top: '65px', transform: 'translate(-60px, 0)' }
  }
  const startDeg = -105
  const endDeg = -15
  const deg = startDeg + ((endDeg - startDeg) * i) / Math.max(1, n - 1)
  const rad = (deg * Math.PI) / 180
  return {
    left: (65 + SUB_RADIUS * Math.cos(rad)) + 'px',
    top: (65 + SUB_RADIUS * Math.sin(rad)) + 'px',
  }
}

const opened = ref(false)
let dragging = false
let moved = false
let ox = 0, oy = 0
function onUp() {
  dragging = false
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('mouseup', onUp)
  if (moved) savePos()
}
function onDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  dragging = true
  moved = false
  ox = e.clientX - pos.x
  oy = e.clientY - pos.y
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
}
function onMove(e) {
  if (!dragging) return
  moved = true
  pos.x = Math.max(40, Math.min(e.clientX - ox, window.innerWidth - 40))
  pos.y = Math.max(50, Math.min(e.clientY - oy, window.innerHeight - 50))
}
function onClick(e) {
  e.stopPropagation()
  if (moved) { moved = false; return }
  opened.value = !opened.value
  emit('toggle', opened.value)
  if (opened.value) emit('open')
}
function handleSub(s) {
  if (s.handler) s.handler()
  else emit('sub', s)
}
defineExpose({ getPos: () => ({ ...pos }), getOpened: () => opened.value })
</script>

<style scoped>
.float-group {
  position: fixed;
  z-index: 200;
  width: 130px;
  height: 130px;
  margin-left: -65px;
  margin-top: -65px;
}
.fab-main {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 56px;
  height: 56px;
  margin-left: -28px;
  margin-top: -28px;
  border-radius: 50%;
  border: none;
  cursor: move;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 22px;
  box-shadow: 0 4px 16px rgba(0,0,0,0.15);
  background: #fff;
  color: #409eff;
  z-index: 3;
  outline: none;
  transition: transform 0.15s, box-shadow 0.15s;
}
.fab-main:hover {
  transform: scale(1.06);
  box-shadow: 0 6px 22px rgba(0,0,0,0.18);
}
.fab-tooltip {
  position: absolute;
  bottom: -27px;
  left: 50%;
  transform: translateX(-50%);
  white-space: nowrap;
  font-size: 10px;
  color: #fff;
  background: rgba(0,0,0,0.72);
  padding: 3px 10px;
  border-radius: 6px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.15s;
}
.fab-main:hover .fab-tooltip { opacity: 1; }

/* 子按钮：围绕主按钮圆心，默认隐藏，.opened 才显示 */
.fab-sub {
  position: absolute;
  left: 50%;
  top: 50%;
  width: 30px;
  height: 30px;
  margin-left: -15px;
  margin-top: -15px;
  border-radius: 50%;
  border: none;
  cursor: pointer;
  background: #fff;
  color: #606266;
  font-size: 13px;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 8px rgba(0,0,0,0.16);
  z-index: 2;
  opacity: 0;
  pointer-events: none;
  transform: translate(0, 0) scale(0.4);
  transition: all 0.25s cubic-bezier(0.34, 1.56, 0.64, 1);
}
.float-group.opened .fab-sub {
  opacity: 1;
  pointer-events: auto;
  transform: translate(0, 0) scale(1);
}
/* 子按钮位置由 JS（subPos）通过 left/top 内联指定 */
.fab-subtip {
  position: absolute;
  right: 40px;
  top: 50%;
  transform: translateY(-50%);
  white-space: nowrap;
  font-size: 10px;
  color: #fff;
  background: rgba(0,0,0,0.78);
  padding: 3px 8px;
  border-radius: 4px;
  opacity: 0;
  pointer-events: none;
}
.fab-sub:hover .fab-subtip { opacity: 1; }
</style>
