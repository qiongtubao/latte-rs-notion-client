<template>
  <div>
    <div class="view-switch">
      <el-radio-group v-model="viewMode">
        <el-radio-button value="board">看板</el-radio-button>
        <el-radio-button value="gantt">甘特图</el-radio-button>
      </el-radio-group>
    </div>

    <template v-if="viewMode === 'board'">
    <!-- 顶部统计 chips -->
    <div class="chips">
      <el-tag
        v-for="s in PROJECT_STATUSES"
        :key="s"
        class="chip"
        effect="plain"
      >
        <span class="dot" :style="{ background: PROJECT_STATUS_COLORS[s] }" />
        {{ s }} {{ grouped[s].length }}
      </el-tag>
      <el-tag class="chip" effect="plain" type="info">共 {{ projects.length }} 项</el-tag>
    </div>

    <!-- 看板 -->
    <div class="board" v-loading="loading">
      <div
        v-for="s in PROJECT_STATUSES"
        :key="s"
        class="column"
        :class="{ 'drag-over': dragOver === s }"
        @dragover.prevent="dragOver = s"
        @dragleave="onDragLeave(s)"
        @drop.prevent="onDrop(s)"
      >
        <div class="column-header">
          <span class="dot" :style="{ background: PROJECT_STATUS_COLORS[s] }" />
          <span class="column-name">{{ s }}</span>
          <span class="column-count">{{ grouped[s].length }}</span>
        </div>

        <div class="column-body">
          <div
            v-for="p in grouped[s]"
            :key="p.id"
            class="card"
            draggable="true"
            @dragstart="onDragStart(p, $event)"
            @dragend="dragOver = null"
            @click="openEdit(p)"
          >
            <div class="card-top">
              <span class="card-name">{{ p.name }}</span>
              <el-button
                class="card-delete"
                text
                type="danger"
                size="small"
                @click.stop="removeProject(p)"
              >删除</el-button>
            </div>
            <div v-if="p.note" class="card-note">{{ p.note }}</div>
            <div v-if="p.start_ts || p.deadline_ts" class="card-dates">
              <span v-if="p.start_ts">{{ fmtDate(p.start_ts) }}</span>
              <span v-if="p.deadline_ts" :class="{ overdue: isOverdue(p) }">
                {{ p.start_ts ? '→ ' : '' }}{{ fmtDate(p.deadline_ts) }}
              </span>
            </div>
          </div>
          <div v-if="grouped[s].length === 0" class="empty-hint">拖拽卡片到这里</div>
        </div>

        <el-button class="add-btn" text @click="openCreate(s)">+ 新建</el-button>
      </div>
    </div>
    </template>

    <GanttChart v-else :projects="projects" v-loading="loading" @edit="openEdit" />

    <!-- 新建/编辑对话框 -->
    <el-dialog v-model="dialog" :title="form.id ? '编辑项目' : '新建项目'" width="480px">
      <el-form label-width="80px">
        <el-form-item label="名称">
          <el-input v-model="form.name" placeholder="项目名称" />
        </el-form-item>
        <el-form-item label="状态">
          <el-select v-model="form.status" style="width: 100%">
            <el-option v-for="s in PROJECT_STATUSES" :key="s" :label="s" :value="s" />
          </el-select>
        </el-form-item>
        <el-form-item label="开始日期">
          <el-date-picker v-model="form.start_ts" type="date" style="width: 100%" value-format="X" />
        </el-form-item>
        <el-form-item label="截止日期">
          <el-date-picker v-model="form.deadline_ts" type="date" style="width: 100%" value-format="X" />
        </el-form-item>
        <el-form-item label="备注">
          <el-input v-model="form.note" type="textarea" :rows="2" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialog = false">取消</el-button>
        <el-button type="primary" :loading="saving" @click="save">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, inject, onMounted, reactive, ref, watch } from 'vue'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from '../api'
import { PROJECT_STATUSES, PROJECT_STATUS_COLORS, fmtDate } from '../utils'
import GanttChart from '../components/GanttChart.vue'

const viewMode = ref('board')
const projects = ref([])
const loading = ref(false)
const dialog = ref(false)
const saving = ref(false)
const form = reactive({ id: null, name: '', status: '待办', start_ts: null, deadline_ts: null, note: '' })

const grouped = computed(() => {
  const map = {}
  for (const s of PROJECT_STATUSES) map[s] = []
  for (const p of projects.value) {
    if (map[p.status]) map[p.status].push(p)
  }
  return map
})

function isOverdue(p) {
  return (
    p.status !== '已完成' &&
    p.deadline_ts &&
    dayjs.unix(p.deadline_ts).endOf('day').isBefore(dayjs())
  )
}

async function load() {
  loading.value = true
  try {
    projects.value = await api.getProjects()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loading.value = false
  }
}

// ---------- 拖拽 ----------

const dragOver = ref(null)
let dragged = null

function onDragStart(p, event) {
  dragged = p
  event.dataTransfer.effectAllowed = 'move'
  event.dataTransfer.setData('text/plain', String(p.id))
}

function onDragLeave(s) {
  if (dragOver.value === s) dragOver.value = null
}

async function onDrop(targetStatus) {
  dragOver.value = null
  const p = dragged
  dragged = null
  if (!p || p.status === targetStatus) return

  const oldStatus = p.status
  p.status = targetStatus // 乐观更新
  try {
    await api.updateProject(p.id, { status: targetStatus })
  } catch (e) {
    p.status = oldStatus
    ElMessage.error(`移动失败：${e.message}`)
    await load() // 刷新恢复原状
  }
}

// ---------- 新建 / 编辑 / 删除 ----------

function openCreate(status) {
  form.id = null
  form.name = ''
  form.status = status
  form.start_ts = null
  form.deadline_ts = null
  form.note = ''
  dialog.value = true
}

function openEdit(p) {
  form.id = p.id
  form.name = p.name
  form.status = p.status && PROJECT_STATUSES.includes(p.status) ? p.status : '待办'
  form.start_ts = p.start_ts ? String(p.start_ts) : null
  form.deadline_ts = p.deadline_ts ? String(p.deadline_ts) : null
  form.note = p.note || ''
  dialog.value = true
}

async function save() {
  if (!form.name.trim()) {
    ElMessage.warning('请填写项目名称')
    return
  }
  saving.value = true
  try {
    const data = {
      name: form.name.trim(),
      status: form.status,
      start_ts: form.start_ts ? Number(form.start_ts) : undefined,
      deadline_ts: form.deadline_ts ? Number(form.deadline_ts) : undefined,
      note: form.note,
    }
    if (form.id) {
      await api.updateProject(form.id, data)
    } else {
      await api.createProject(data)
    }
    dialog.value = false
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}

async function removeProject(p) {
  try {
    await ElMessageBox.confirm(`删除项目「${p.name}」？`, '确认删除', { type: 'warning' })
  } catch {
    return
  }
  try {
    await api.deleteProject(p.id)
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}
const subAction = inject('subAction')
watch(subAction, (act) => {
  if (!act || act.key !== 'projects') return
  if (act.action === 'board') {
    viewMode.value = 'board'
  } else if (act.action === 'gantt') {
    viewMode.value = 'gantt'
  }
})

onMounted(load)
</script>

<style scoped>
.view-switch {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 12px;
}

.chips {
  display: flex;
  gap: 8px;
  margin-bottom: 14px;
  flex-wrap: wrap;
}

.chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.board {
  display: flex;
  gap: 12px;
  overflow-x: auto;
  padding-bottom: 12px;
  align-items: flex-start;
}

.column {
  flex: 0 0 260px;
  background: #f0f2f5;
  border-radius: 10px;
  border: 2px solid transparent;
  display: flex;
  flex-direction: column;
  min-height: 220px;
  transition: border-color 0.15s, background 0.15s;
}

.column.drag-over {
  border-color: #409eff;
  background: #ecf5ff;
}

.column-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 14px 8px;
  font-weight: 600;
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  display: inline-block;
  flex: none;
}

.column-count {
  margin-left: auto;
  color: #909399;
  font-size: 13px;
  font-weight: 400;
}

.column-body {
  flex: 1;
  padding: 0 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.card {
  background: #fff;
  border-radius: 8px;
  padding: 10px 12px;
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
  cursor: grab;
}

.card:active {
  cursor: grabbing;
}

.card-top {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 6px;
}

.card-name {
  font-weight: 600;
  word-break: break-all;
}

.card-delete {
  visibility: hidden;
  flex: none;
  padding: 0;
  height: auto;
}

.card:hover .card-delete {
  visibility: visible;
}

.card-note {
  margin-top: 6px;
  font-size: 13px;
  color: #606266;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.card-dates {
  margin-top: 8px;
  font-size: 12px;
  color: #909399;
  display: flex;
  gap: 4px;
}

.card-dates .overdue {
  color: #f56c6c;
  font-weight: 600;
}

.empty-hint {
  text-align: center;
  color: #c0c4cc;
  font-size: 13px;
  padding: 24px 0;
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
}

.add-btn {
  margin: 8px 10px 10px;
  color: #909399;
}
</style>
