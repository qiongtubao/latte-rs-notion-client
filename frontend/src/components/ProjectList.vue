<template>
  <div class="pll" v-loading="loading">
    <div class="pll-header">
      <span class="pll-title">项目（{{ rows.length }}）</span>
      <span class="pll-view">
        <span :class="{ on: view === 'list' }" @click="view = 'list'">列表</span>
        <span :class="{ on: view === 'gantt' }" @click="view = 'gantt'">甘特</span>
      </span>
      <span class="pll-open" @click="openFull">打开看板 →</span>
    </div>

    <!-- 甘特图视图 -->
    <div v-if="view === 'gantt'" class="pll-gantt">
      <GanttChart :projects="projects" />
    </div>

    <template v-else>
      <!-- 空态 -->
      <div v-if="!loading && rows.length === 0" class="pll-empty">没有进行中的项目</div>

      <div v-for="p in rows" :key="p.id" class="pll-block">
        <div
          class="pll-row"
          :style="{ borderLeftColor: statusColor(p.status) }"
        >
          <span class="pll-arrow" @click="toggleExpand(p.id)">
            {{ expanded.has(p.id) ? '▼' : '▶' }}
          </span>
          <!-- 状态标签：点击可修改 -->
          <el-dropdown trigger="click" @command="(s) => setStatus(p, s)">
            <span class="pll-status clickable" :style="{ background: statusColor(p.status) }">
              {{ p.status }}
            </span>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item v-for="s in PROJECT_STATUSES" :key="s" :command="s">
                  <span class="pll-status" :style="{ background: statusColor(s) }">{{ s }}</span>
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          <span class="pll-name" :title="p.name" @click="toggleExpand(p.id)">{{ p.name }}</span>
          <span class="pll-taskcount" :class="{ none: tasksOf(p.id).length === 0 }">
            {{ tasksOf(p.id).length }} 任务
          </span>
          <span v-if="p.deadline_ts" class="pll-deadline" :class="{ near: isNear(p.deadline_ts) }">
            🕐 {{ fmtDeadline(p.deadline_ts) }}
          </span>
        </div>

        <!-- 项目详情 + 关联任务（可勾选完成） -->
        <div v-if="expanded.has(p.id)" class="pll-tasks">
          <!-- 项目详细内容 -->
          <div class="pll-detail">
            <span class="pll-dates">
              📅 {{ p.start_ts ? fmtDeadline(p.start_ts) : '未排期' }}
              ~ {{ p.deadline_ts ? fmtDeadline(p.deadline_ts) : '未排期' }}
            </span>
            <span v-if="p.note" class="pll-note" :title="p.note">📝 {{ p.note }}</span>
          </div>
          <div v-if="tasksOf(p.id).length === 0" class="pll-notask">没有关联任务</div>
          <div v-for="t in tasksOf(p.id)" :key="t.id" class="pll-task">
            <div class="pll-task-line1">
              <el-checkbox :model-value="t.done" @change="toggleTaskDone(t)" />
              <span class="pll-task-tag" :style="{ background: quadOf(t).color }">{{ quadOf(t).label }}</span>
              <span class="pll-task-title" :title="t.title">{{ t.title }}</span>
            </div>
            <!-- 任务详细内容 -->
            <div v-if="hasTaskMeta(t)" class="pll-task-line2">
              <span v-if="t.task_type" class="pll-meta pll-type">{{ t.task_type }}</span>
              <span v-if="t.estimated_minutes" class="pll-meta">预估{{ t.estimated_minutes }}m</span>
              <span v-if="t.start_ts" class="pll-meta" :class="{ future: t.start_ts > nowTs }">
                🕐 {{ fmtStart(t.start_ts) }}
              </span>
              <span v-if="t.executed_secs > 0" class="pll-meta pll-exec">
                ⏱ 累计{{ fmtDur(t.executed_secs) }}
              </span>
            </div>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup>
import { computed, inject, onMounted, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'
import { PROJECT_STATUSES, PROJECT_STATUS_COLORS } from '../utils'
import GanttChart from './GanttChart.vue'

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

const projects = ref([])
const tasks = ref([])
const loading = ref(false)
const view = ref('list')
const expanded = ref(new Set())

function statusColor(s) {
  return PROJECT_STATUS_COLORS[s] || '#909399'
}

// 未完成的项目：进行中 > 待办 > 暂存 > 暂停，同级按截止时间
const ORDER = { 进行中: 0, 待办: 1, 暂存: 2, 暂停: 3 }
const rows = computed(() =>
  projects.value
    .filter((p) => p.status !== '已完成')
    .slice()
    .sort(
      (a, b) =>
        (ORDER[a.status] ?? 9) - (ORDER[b.status] ?? 9) ||
        (a.deadline_ts || 9e12) - (b.deadline_ts || 9e12)
    )
)

// 项目关联的未完成任务（按优先级排序）
function tasksOf(projectId) {
  return tasks.value
    .filter((t) => t.project_id === projectId)
    .slice()
    .sort((a, b) => quadOf(a).order - quadOf(b).order)
}

function toggleExpand(id) {
  const s = new Set(expanded.value)
  if (s.has(id)) s.delete(id)
  else s.add(id)
  expanded.value = s
}

async function toggleTaskDone(t) {
  try {
    await api.updateTask(t.id, { done: true })
    tasks.value = tasks.value.filter((x) => x.id !== t.id)
  } catch (e) {
    ElMessage.error(e.message)
  }
}

function fmtDeadline(ts) {
  return dayjs.unix(ts).format('MM-DD')
}
// 截止在 2 天内或已过期：红色提醒（与派生提醒窗口一致）
function isNear(ts) {
  return dayjs.unix(ts).isBefore(dayjs().add(2, 'day').endOf('day'))
}

const nowTs = ref(Math.floor(Date.now() / 1000))
function fmtStart(ts) {
  return dayjs.unix(ts).format('MM-DD HH:mm')
}
function fmtDur(secs) {
  if (secs < 60) return `${secs}s`
  const h = Math.floor(secs / 3600)
  const m = Math.round((secs % 3600) / 60)
  return h > 0 ? `${h}h${m}m` : `${m}m`
}
function hasTaskMeta(t) {
  return t.task_type || t.estimated_minutes || t.start_ts || t.executed_secs > 0
}

// 打开完整项目面板（由 App.vue provide）
const openPanel = inject('openPanel', null)
function openFull() {
  if (openPanel) openPanel('projects')
}

async function load() {
  loading.value = true
  try {
    const [ps, ts] = await Promise.all([api.getProjects(), api.getUnfinishedTasks()])
    projects.value = ps
    tasks.value = ts
    nowTs.value = Math.floor(Date.now() / 1000)
    // 有关联任务的项目默认展开
    const withTasks = new Set(ts.filter((t) => t.project_id).map((t) => t.project_id))
    expanded.value = withTasks
  } catch (e) {
    ElMessage.error(`加载项目失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

async function setStatus(p, status) {
  if (p.status === status) return
  try {
    await api.updateProject(p.id, { status })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.pll {
  min-height: 80px;
}
.pll-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.pll-title {
  font-size: 13px;
  font-weight: 600;
  color: #303133;
  flex: 1;
}
.pll-view {
  display: flex;
  gap: 8px;
  font-size: 12px;
}
.pll-view span {
  cursor: pointer;
  color: #c0c4cc;
}
.pll-view span.on {
  color: #e6a23c;
  font-weight: 600;
}
.pll-open {
  font-size: 12px;
  color: #409eff;
  cursor: pointer;
  flex-shrink: 0;
}
.pll-open:hover {
  text-decoration: underline;
}
.pll-empty {
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
  color: #909399;
  font-size: 13px;
  text-align: center;
  padding: 24px 0;
}
.pll-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 7px 10px;
  margin-bottom: 4px;
  background: #fff;
  border: 1px solid #ebeef5;
  border-left: 3px solid #909399;
  border-radius: 6px;
}
.pll-arrow {
  flex-shrink: 0;
  font-size: 8px;
  color: #909399;
  cursor: pointer;
  width: 12px;
}
.pll-status {
  flex-shrink: 0;
  color: #fff;
  font-size: 10px;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 4px;
}
.pll-status.clickable {
  cursor: pointer;
}
.pll-status.clickable:hover {
  filter: brightness(1.1);
}
.pll-name {
  flex: 1;
  font-size: 13px;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
}
.pll-taskcount {
  flex-shrink: 0;
  font-size: 11px;
  color: #409eff;
}
.pll-taskcount.none {
  color: #c0c4cc;
}
.pll-deadline {
  flex-shrink: 0;
  font-size: 11px;
  color: #909399;
}
.pll-deadline.near {
  color: #f56c6c;
}
.pll-tasks {
  margin: 0 0 8px 18px;
  padding: 6px 8px;
  background: #f8f9fb;
  border-radius: 6px;
}
.pll-notask {
  font-size: 12px;
  color: #c0c4cc;
  text-align: center;
  padding: 6px 0;
}
.pll-task {
  padding: 3px 0;
}
.pll-task-line1 {
  display: flex;
  align-items: center;
  gap: 6px;
}
.pll-task-line2 {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 10px;
  margin: 2px 0 2px 24px;
}
.pll-meta {
  font-size: 11px;
  color: #909399;
}
.pll-type {
  color: #13c2c2;
}
.pll-meta.future {
  color: #e6a23c;
}
.pll-exec {
  color: #67c23a;
}
.pll-detail {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  font-size: 11px;
  color: #909399;
  margin-bottom: 6px;
  padding-bottom: 6px;
  border-bottom: 1px dashed #e4e7ed;
}
.pll-note {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 100%;
}
.pll-task-tag {
  flex-shrink: 0;
  color: #fff;
  font-size: 10px;
  line-height: 1;
  padding: 2px 5px;
  border-radius: 4px;
}
.pll-task-title {
  flex: 1;
  font-size: 12px;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pll-gantt {
  overflow-x: auto;
}
</style>
