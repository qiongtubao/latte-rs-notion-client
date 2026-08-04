<template>
  <div class="ttl" v-loading="loading">
    <!-- 内联添加行：点子按钮「添加」出现 -->
    <div v-if="showAdd" class="ttl-add">
      <el-input
        ref="addInputRef"
        v-model="newTitle"
        size="small"
        placeholder="任务标题，回车添加"
        @keyup.enter="quickAdd"
        @keyup.esc="showAdd = false"
      />
      <el-button size="small" type="primary" :loading="adding" @click="quickAdd">添加</el-button>
    </div>

    <!-- 空态：没任务也有个框 -->
    <div v-if="!loading && sortedTasks.length === 0" class="ttl-empty">
      没有未完成的任务
    </div>

    <div
      v-for="t in sortedTasks"
      :key="t.id"
      class="ttl-row"
      :style="{ borderLeftColor: quadOf(t).color }"
    >
      <div class="ttl-line1">
        <el-checkbox :model-value="t.done" @change="toggleDone(t, true)" />
        <span class="ttl-tag" :style="{ background: quadOf(t).color }">{{ quadOf(t).label }}</span>
        <span class="ttl-title" :title="t.title">{{ t.title }}</span>
        <span v-if="t.estimated_minutes" class="ttl-est">预估{{ t.estimated_minutes }}m</span>
      </div>
      <!-- 元信息行：类型 / 项目 / 计划开始 / 累计执行 -->
      <div v-if="hasMeta(t)" class="ttl-line2">
        <span v-if="t.task_type" class="ttl-meta ttl-type">{{ t.task_type }}</span>
        <span v-if="projectName(t)" class="ttl-meta">📁 {{ projectName(t) }}</span>
        <span v-if="t.start_ts" class="ttl-meta" :class="{ future: t.start_ts > nowTs }">
          🕐 {{ fmtStart(t.start_ts) }}
        </span>
        <span v-if="t.executed_secs > 0" class="ttl-meta ttl-exec">
          ⏱ 累计{{ fmtDur(t.executed_secs) }}
        </span>
      </div>
    </div>

    <!-- 已完成：点子按钮「完成」显示/隐藏 -->
    <template v-if="showDone && doneTasks.length > 0">
      <div class="ttl-done-header">已完成（{{ doneTasks.length }}）</div>
      <div v-for="t in doneTasks" :key="t.id" class="ttl-row done">
        <div class="ttl-line1">
          <el-checkbox :model-value="t.done" @change="toggleDone(t, false)" />
          <span class="ttl-title done-title">{{ t.title }}</span>
          <span v-if="t.executed_secs > 0" class="ttl-meta ttl-exec">⏱ {{ fmtDur(t.executed_secs) }}</span>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { computed, inject, nextTick, onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'

// 四象限优先级：颜色标识
const QUADS = {
  q1: { label: '重要紧急', color: '#f56c6c', order: 0 },
  q2: { label: '重要', color: '#e6a23c', order: 1 },
  q3: { label: '紧急', color: '#409eff', order: 2 },
  q4: { label: '普通', color: '#909399', order: 3 },
}
function quadOf(t) {
  if (t.important && t.urgent) return QUADS.q1
  if (t.important) return QUADS.q2
  if (t.urgent) return QUADS.q3
  return QUADS.q4
}

// 排序方式：子按钮「排序」循环切换
const SORTS = ['priority', 'start', 'created']
const sortMode = ref('priority')

const tasks = ref([])
const todayTasks = ref([])
const projects = ref([])
const loading = ref(false)
const nowTs = ref(Math.floor(Date.now() / 1000))

// 内联添加
const showAdd = ref(false)
const newTitle = ref('')
const adding = ref(false)
const addInputRef = ref(null)

// 已完成展示
const showDone = ref(false)

const sortedTasks = computed(() => {
  const list = tasks.value.slice()
  if (sortMode.value === 'start') {
    list.sort((a, b) => (a.start_ts || 9e12) - (b.start_ts || 9e12))
  } else if (sortMode.value === 'created') {
    list.sort((a, b) => a.created_ts - b.created_ts)
  } else {
    list.sort((a, b) => quadOf(a).order - quadOf(b).order || (a.start_ts || 9e12) - (b.start_ts || 9e12))
  }
  return list
})
const doneTasks = computed(() => todayTasks.value.filter((t) => t.done))

const projectMap = computed(() => new Map(projects.value.map((p) => [p.id, p.name])))
function projectName(t) {
  return t.project_id ? projectMap.value.get(t.project_id) || '' : ''
}
function hasMeta(t) {
  return t.task_type || projectName(t) || t.start_ts || t.executed_secs > 0
}
function fmtStart(ts) {
  return dayjs.unix(ts).format('MM-DD HH:mm')
}
function fmtDur(secs) {
  const h = Math.floor(secs / 3600)
  const m = Math.round((secs % 3600) / 60)
  return h > 0 ? `${h}h${m}m` : `${m}m`
}

async function load() {
  loading.value = true
  try {
    const reqs = [api.getUnfinishedTasks(), api.getProjects()]
    if (showDone.value) reqs.push(api.getTasks())
    const [ts, ps, today] = await Promise.all(reqs)
    tasks.value = ts
    projects.value = ps
    if (today) todayTasks.value = today
    nowTs.value = Math.floor(Date.now() / 1000)
  } catch (e) {
    ElMessage.error(`加载任务失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

async function quickAdd() {
  const title = newTitle.value.trim()
  if (!title) return
  adding.value = true
  try {
    await api.createTask({ title })
    newTitle.value = ''
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adding.value = false
  }
}

async function toggleDone(t, done) {
  try {
    await api.updateTask(t.id, { done })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

// 子按钮动作：＋添加 / ▲排序 / ✓完成（由悬浮按钮经 provide 下发）
const subAction = inject('subAction')
watch(subAction, (act) => {
  if (!act || act.key !== 'today') return
  if (act.action === 'add') {
    showAdd.value = !showAdd.value
    if (showAdd.value) nextTick(() => addInputRef.value?.focus())
  } else if (act.action === 'sort') {
    const idx = SORTS.indexOf(sortMode.value)
    sortMode.value = SORTS[(idx + 1) % SORTS.length]
    const names = { priority: '按优先级', start: '按计划开始', created: '按创建时间' }
    ElMessage.success(`排序：${names[sortMode.value]}`)
  } else if (act.action === 'done') {
    showDone.value = !showDone.value
    if (showDone.value) load()
  }
})

onMounted(load)
</script>

<style scoped>
.ttl {
  min-height: 80px;
}
.ttl-add {
  display: flex;
  gap: 6px;
  margin-bottom: 8px;
}
.ttl-empty {
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
  color: #909399;
  font-size: 13px;
  text-align: center;
  padding: 24px 0;
}
.ttl-row {
  padding: 7px 10px;
  margin-bottom: 6px;
  background: #fff;
  border: 1px solid #ebeef5;
  border-left: 3px solid #909399;
  border-radius: 6px;
}
.ttl-row.done {
  opacity: 0.6;
  border-left-color: #dcdfe6;
}
.ttl-line1 {
  display: flex;
  align-items: center;
  gap: 8px;
}
.ttl-tag {
  flex-shrink: 0;
  color: #fff;
  font-size: 10px;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 4px;
}
.ttl-title {
  flex: 1;
  font-size: 13px;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.done-title {
  text-decoration: line-through;
  color: #909399;
}
.ttl-est {
  flex-shrink: 0;
  font-size: 11px;
  color: #909399;
}
.ttl-line2 {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  margin-top: 4px;
  padding-left: 24px;
}
.ttl-meta {
  font-size: 11px;
  color: #909399;
}
.ttl-type {
  color: #13c2c2;
}
.ttl-meta.future {
  color: #e6a23c;
}
.ttl-exec {
  color: #67c23a;
}
.ttl-done-header {
  font-size: 12px;
  color: #909399;
  margin: 10px 0 6px;
}
</style>
