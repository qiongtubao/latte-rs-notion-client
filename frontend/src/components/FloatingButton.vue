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

    <!-- 子按钮：左侧分布，点击主按钮后显示 -->
    <button
      v-for="(s, i) in subs"
      :key="i"
      class="fab-sub"
      :class="s.pos || ('p' + (i + 1))"
      :title="s.label"
      @click.stop="handleSub(s)"
    >
      {{ s.icon }}
      <span class="fab-subtip">{{ s.label }}</span>
    </button>
  </div>
</template>

<script setup>
import { computed, ref } from 'vue'

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
      if (typeof p.x === 'number' && typeof p.y === 'number') return p
    }
  } catch { /* 忽略损坏数据 */ }
  return { x: props.x, y: props.y ?? window.innerHeight - 120 }
}
const pos = ref(loadPos())
function savePos() {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify({ x: pos.value.x, y: pos.value.y }))
  } catch { /* 忽略存储失败 */ }
}

const opened = ref(false)
let dragging = false
let moved = false
let ox = 0, oy = 0
function onUp() {
  dragging = false
  window.removeEventListener('mousemove', onMove)
  window.removeEventListener('mouseup', onUp)
  if (moved) savePos()   // 拖拽过才保存位置
}
function onDown(e) {
  if (e.button !== 0) return
  e.preventDefault()
  dragging = true
  moved = false
  ox = e.clientX - pos.value.x
  oy = e.clientY - pos.value.y
  window.addEventListener('mousemove', onMove)
  window.addEventListener('mouseup', onUp)
}
function onMove(e) {
  if (!dragging) return
  moved = true
  pos.value.x = Math.max(40, Math.min(e.clientX - ox, window.innerWidth - 40))
  pos.value.y = Math.max(50, Math.min(e.clientY - oy, window.innerHeight - 40))
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
// 供父组件读取是否已打开
defineExpose({ getPos: () => ({ ...pos.value }), getOpened: () => opened.value })
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
/* 左侧分布位置 */
.fab-sub.p1 { left: 8%; }                       /* 上 */
.fab-sub.p2 { left: 8%; top: 16%; }              /* 左上 */
.fab-sub.p3 { left: -4%; top: 50%; }             /* 左 */
.fab-sub.p4 { left: 8%; top: 84%; }              /* 左下 */
.fab-sub.p5 { left: 50%; top: 100%; }            /* 下 */
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
