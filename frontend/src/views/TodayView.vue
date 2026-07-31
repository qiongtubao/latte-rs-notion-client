<template>
  <div>
    <!-- 日期导航 + 进度 -->
    <el-card shadow="never" class="head-card">
      <div class="head-row">
        <div class="date-nav">
          <el-button-group>
            <el-button @click="shiftDay(-1)">←</el-button>
            <el-button @click="goToday">今天</el-button>
            <el-button @click="shiftDay(1)">→</el-button>
          </el-button-group>
          <span class="date-title">
            {{ dayTitle }}
            <el-tag v-if="isToday" type="primary" size="small" effect="dark">今天</el-tag>
          </span>
        </div>
        <div class="progress-box">
          <el-progress
            type="circle"
            :width="56"
            :percentage="percent"
            :status="percent === 100 && tasks.length > 0 ? 'success' : undefined"
          />
          <span class="progress-text">已完成 {{ doneCount }}/{{ tasks.length }}</span>
        </div>
      </div>
    </el-card>

    <!-- 添加行 -->
    <el-card shadow="never" class="add-card">
      <div class="add-row">
        <el-input
          v-model="newTitle"
          placeholder="今天要做什么？"
          @keyup.enter="addTask"
        />
        <el-select v-model="newPriority" style="width: 90px">
          <el-option v-for="p in PRIORITIES" :key="p" :label="p" :value="p" />
        </el-select>
        <el-button type="primary" :loading="adding" @click="addTask">添加</el-button>
      </div>
    </el-card>

    <!-- 任务列表 -->
    <el-card shadow="never" v-loading="loading">
      <el-empty v-if="tasks.length === 0 && !loading" :image-size="80">
        <template #description>
          <span v-if="isToday">今天还没有任务，添加一个吧</span>
          <span v-else>
            这一天没有任务，
            <el-link type="primary" @click="goToday">回到今天</el-link>
          </span>
        </template>
      </el-empty>

      <template v-else>
        <div
          v-for="task in pendingTasks"
          :key="task.id"
          class="task-row"
        >
          <el-checkbox :model-value="task.done" @change="toggleDone(task, true)" />
          <span class="task-title">{{ task.title }}</span>
          <el-tag :type="PRIORITY_TYPES[task.priority] || 'info'" size="small">
            {{ task.priority || '中' }}
          </el-tag>
          <span class="task-ops">
            <el-icon title="开始计时" @click="startTiming(task)"><VideoPlay /></el-icon>
            <el-icon title="编辑" @click="openEdit(task)"><EditPen /></el-icon>
            <el-icon title="删除" class="op-danger" @click="removeTask(task)"><Delete /></el-icon>
          </span>
        </div>

        <!-- 已完成（可折叠） -->
        <div v-if="doneTasks.length > 0" class="done-section">
          <div class="done-header" @click="doneCollapsed = !doneCollapsed">
            <el-icon class="collapse-icon" :class="{ collapsed: doneCollapsed }"><ArrowDown /></el-icon>
            已完成 {{ doneTasks.length }}
          </div>
          <template v-if="!doneCollapsed">
            <div
              v-for="task in doneTasks"
              :key="task.id"
              class="task-row done"
            >
              <el-checkbox :model-value="task.done" @change="toggleDone(task, false)" />
              <span class="task-title">{{ task.title }}</span>
              <el-tag :type="PRIORITY_TYPES[task.priority] || 'info'" size="small">
                {{ task.priority || '中' }}
              </el-tag>
              <span class="task-ops">
                <el-icon title="编辑" @click="openEdit(task)"><EditPen /></el-icon>
                <el-icon title="删除" class="op-danger" @click="removeTask(task)"><Delete /></el-icon>
              </span>
            </div>
          </template>
        </div>
      </template>
    </el-card>

    <!-- 编辑弹窗 -->
    <el-dialog v-model="editDialog" title="编辑任务" width="420px">
      <el-form label-width="70px">
        <el-form-item label="标题">
          <el-input v-model="editForm.title" @keyup.enter="saveEdit" />
        </el-form-item>
        <el-form-item label="优先级">
          <el-select v-model="editForm.priority" style="width: 100%">
            <el-option v-for="p in PRIORITIES" :key="p" :label="p" :value="p" />
          </el-select>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editDialog = false">取消</el-button>
        <el-button type="primary" :loading="saving" @click="saveEdit">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { ArrowDown, Delete, EditPen, VideoPlay } from '@element-plus/icons-vue'
import { api } from '../api'

const PRIORITIES = ['高', '中', '低']
const PRIORITY_TYPES = { 高: 'danger', 中: 'warning', 低: 'info' }
const WEEKDAYS = ['日', '一', '二', '三', '四', '五', '六']

const date = ref(dayjs().format('YYYY-MM-DD'))
const tasks = ref([])
const loading = ref(false)

const newTitle = ref('')
const newPriority = ref('中')
const adding = ref(false)

const doneCollapsed = ref(false)

const editDialog = ref(false)
const saving = ref(false)
const editForm = reactive({ id: null, title: '', priority: '中' })

const isToday = computed(() => date.value === dayjs().format('YYYY-MM-DD'))

const dayTitle = computed(() => {
  const d = dayjs(date.value)
  return `${d.month() + 1}月${d.date()}日 星期${WEEKDAYS[d.day()]}`
})

const pendingTasks = computed(() => tasks.value.filter((t) => !t.done))
const doneTasks = computed(() => tasks.value.filter((t) => t.done))
const doneCount = computed(() => doneTasks.value.length)
const percent = computed(() =>
  tasks.value.length === 0 ? 0 : Math.round((doneCount.value / tasks.value.length) * 100)
)

async function load() {
  loading.value = true
  try {
    tasks.value = await api.getTasks(date.value)
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loading.value = false
  }
}

function shiftDay(delta) {
  date.value = dayjs(date.value).add(delta, 'day').format('YYYY-MM-DD')
  load()
}

function goToday() {
  date.value = dayjs().format('YYYY-MM-DD')
  load()
}

async function addTask() {
  const title = newTitle.value.trim()
  if (!title) {
    ElMessage.warning('先输入任务标题')
    return
  }
  adding.value = true
  try {
    await api.createTask({ title, priority: newPriority.value, date: date.value })
    newTitle.value = ''
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adding.value = false
  }
}

async function toggleDone(task, done) {
  try {
    await api.updateTask(task.id, { done })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

async function startTiming(task) {
  try {
    await api.startEvent({ content: task.title })
    ElMessage.success('已开始计时，可到「时间碎片」查看进行中事件')
  } catch (e) {
    if (e.status === 409) {
      ElMessage.warning('已有进行中的事件')
    } else {
      ElMessage.error(e.message)
    }
  }
}

function openEdit(task) {
  editForm.id = task.id
  editForm.title = task.title
  editForm.priority = PRIORITIES.includes(task.priority) ? task.priority : '中'
  editDialog.value = true
}

async function saveEdit() {
  if (!editForm.title.trim()) {
    ElMessage.warning('标题不能为空')
    return
  }
  saving.value = true
  try {
    await api.updateTask(editForm.id, {
      title: editForm.title.trim(),
      priority: editForm.priority,
    })
    editDialog.value = false
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}

async function removeTask(task) {
  try {
    await ElMessageBox.confirm(`删除任务「${task.title}」？`, '确认删除', { type: 'warning' })
  } catch {
    return
  }
  try {
    await api.deleteTask(task.id)
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.head-card {
  margin-bottom: 14px;
}

.head-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 12px;
}

.date-nav {
  display: flex;
  align-items: center;
  gap: 12px;
}

.date-title {
  font-size: 17px;
  font-weight: 600;
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.progress-box {
  display: flex;
  align-items: center;
  gap: 10px;
}

.progress-text {
  color: #606266;
  font-size: 13px;
}

.add-card {
  margin-bottom: 14px;
}

.add-row {
  display: flex;
  gap: 10px;
}

.task-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 8px;
  border-radius: 8px;
}

.task-row:hover {
  background: #f5f6fa;
}

.task-row.done {
  opacity: 0.5;
}

.task-row.done .task-title {
  text-decoration: line-through;
}

.task-title {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.task-ops {
  display: none;
  gap: 8px;
  color: #909399;
}

.task-row:hover .task-ops {
  display: inline-flex;
}

.task-ops .el-icon:hover {
  color: #409eff;
}

.task-ops .op-danger:hover {
  color: #f56c6c;
}

.done-section {
  margin-top: 10px;
  border-top: 1px dashed #e4e7ed;
  padding-top: 8px;
}

.done-header {
  display: flex;
  align-items: center;
  gap: 6px;
  color: #909399;
  font-size: 13px;
  cursor: pointer;
  padding: 4px 8px;
  user-select: none;
}

.collapse-icon {
  transition: transform 0.2s;
}

.collapse-icon.collapsed {
  transform: rotate(-90deg);
}
</style>
