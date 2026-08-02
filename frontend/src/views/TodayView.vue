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
          <el-button size="small" @click="manualRollover" :disabled="rolling">
            {{ rolling ? '移入中...' : '移入今日' }}
          </el-button>
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
        <el-checkbox v-model="newImportant">重要</el-checkbox>
        <el-checkbox v-model="newUrgent">紧急</el-checkbox>
        <el-select v-model="newPriority" style="width: 90px">
          <el-option v-for="p in PRIORITIES" :key="p" :label="p" :value="p" />
        </el-select>
        <el-select v-model="newEstimate" style="width: 100px" placeholder="预估">
          <el-option label="无预估" :value="null" />
          <el-option v-for="m in [15,30,45,60,90,120]" :key="m" :label="m + 'min'" :value="m" />
        </el-select>
        <el-button type="primary" :loading="adding" @click="addTask">添加</el-button>
      </div>
    </el-card>

    <!-- 四象限筛选 + 进行中事件 -->
    <el-card shadow="never" class="filter-card">
      <el-radio-group v-model="quadrantFilter" size="small">
        <el-radio-button value="all">全部</el-radio-button>
        <el-radio-button value="q1">重要+紧急</el-radio-button>
        <el-radio-button value="q2">重要不紧急</el-radio-button>
        <el-radio-button value="q3">紧急不重要</el-radio-button>
        <el-radio-button value="q4">都不</el-radio-button>
      </el-radio-group>
      <el-tag size="small" type="warning" style="margin-left: 12px" v-if="derivedCount > 0">
        项目截止提醒 {{ derivedCount }}
      </el-tag>
      <el-tag v-if="ongoingEvent" size="small" type="success" style="margin-left: 8px">
        ⏱ {{ ongoingEvent.content || '进行中' }} {{ ongoingDuration }}
      </el-tag>
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
          :class="{ derived: isDerived(task), expanded: expandedTask === task.id }"
        >
          <el-checkbox :model-value="task.done" @change="toggleDone(task, true)" />
          <span class="task-title" @click="toggleExpand(task)">{{ task.title }}</span>
          <el-tag v-if="task.important && task.urgent" type="danger" size="small">重要紧急</el-tag>
          <el-tag v-else-if="task.important" type="warning" size="small">重要</el-tag>
          <el-tag v-else-if="task.urgent" type="primary" size="small">紧急</el-tag>
          <el-tag v-if="task.estimated_minutes" size="small" type="info">
            {{ task.estimated_minutes }}min
          </el-tag>
          <el-tag v-if="task.pomodoro_count > 0" size="small" type="success">
            🍅 {{ task.pomodoro_count }}
          </el-tag>
          <span class="task-ops">
            <template v-if="isDerived(task)">
              <el-icon title="项目派生提醒"><Bell /></el-icon>
            </template>
            <template v-else>
              <el-icon title="番茄钟 25min" @click="startPomodoro(task)"><Timer /></el-icon>
              <el-icon title="开始计时" @click="startTiming(task)"><VideoPlay /></el-icon>
              <el-icon title="编辑" @click="openEdit(task)"><EditPen /></el-icon>
              <el-icon title="删除" class="op-danger" @click="removeTask(task)"><Delete /></el-icon>
            </template>
          </span>
          <!-- 展开区域：备注/子任务 -->
          <div v-if="expandedTask === task.id" class="task-expand">
            <el-input
              v-model="editNotes"
              type="textarea"
              :rows="3"
              placeholder="备注或子任务（- [ ] 子任务 / - [x] 已完成）"
              @blur="saveNotes(task)"
            />
          </div>
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
              <el-tag v-if="task.estimated_minutes" size="small" type="info">
                {{ task.estimated_minutes }}min
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

    <!-- 今日时间轴 -->
    <el-card shadow="never" class="timeline-card" v-if="isToday">
      <template #header>
        <span>今日时间轴</span>
        <el-button size="small" text @click="loadTimeline">刷新</el-button>
      </template>
      <div v-if="timeline.length === 0" class="timeline-empty">今天还没有时间记录</div>
      <div v-else class="timeline">
        <div v-for="ev in timeline" :key="ev.id" class="timeline-item">
          <div class="timeline-dot" :class="evTagClass(ev.tag)" />
          <div class="timeline-body">
            <div class="timeline-time">{{ fmtTime(ev.start_ts) }}{{ ev.end_ts ? ' - ' + fmtTime(ev.end_ts) : ' 进行中' }}</div>
            <div class="timeline-content">{{ ev.content || '(无内容)' }}</div>
            <el-tag size="small" :type="evTagType(ev.tag)">{{ ev.tag }}</el-tag>
          </div>
        </div>
      </div>
    </el-card>

    <!-- 快速记账 -->
    <el-card shadow="never" class="expense-card" v-if="isToday">
      <template #header>快速记账</template>
      <div class="expense-row">
        <el-input v-model="expenseItem" placeholder="买了什么" style="width: 160px" />
        <el-input-number v-model="expenseAmount" :min="0" :precision="2" :step="10" style="width: 120px" />
        <el-select v-model="expenseCategory" style="width: 100px">
          <el-option v-for="c in CATEGORIES" :key="c" :label="c" :value="c" />
        </el-select>
        <el-button type="primary" size="small" :loading="expenseAdding" @click="addExpense">记账</el-button>
      </div>
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
        <el-form-item label="重要">
          <el-switch v-model="editForm.important" />
        </el-form-item>
        <el-form-item label="紧急">
          <el-switch v-model="editForm.urgent" />
        </el-form-item>
        <el-form-item label="预估">
          <el-select v-model="editForm.estimated_minutes" style="width: 100%" placeholder="无">
            <el-option label="无" :value="null" />
            <el-option v-for="m in [15,30,45,60,90,120]" :key="m" :label="m + ' 分钟'" :value="m" />
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
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { ArrowDown, Bell, Delete, EditPen, Timer, VideoPlay } from '@element-plus/icons-vue'
import { api } from '../api'

const PRIORITIES = ['高', '中', '低']
const PRIORITY_TYPES = { 高: 'danger', 中: 'warning', 低: 'info' }
const CATEGORIES = ['餐饮', '交通', '购物', '娱乐', '医疗', '教育', '其他']
const WEEKDAYS = ['日', '一', '二', '三', '四', '五', '六']

const date = ref(dayjs().format('YYYY-MM-DD'))
const tasks = ref([])
const loading = ref(false)
const rolledOverToday = ref(false)
const rolling = ref(false)

const newTitle = ref('')
const newPriority = ref('中')
const newImportant = ref(false)
const newUrgent = ref(false)
const newEstimate = ref(null)
const adding = ref(false)

const doneCollapsed = ref(false)
const quadrantFilter = ref('all')
const expandedTask = ref(null)
const editNotes = ref('')

const editDialog = ref(false)
const saving = ref(false)
const editForm = reactive({ id: null, title: '', priority: '中', important: false, urgent: false, estimated_minutes: null })

// 进行中事件
const ongoingEvent = ref(null)
const ongoingDuration = ref('')
let ongoingTimer = null

// 今日时间轴
const timeline = ref([])

// 快速记账
const expenseItem = ref('')
const expenseAmount = ref(0)
const expenseCategory = ref('餐饮')
const expenseAdding = ref(false)

const isToday = computed(() => date.value === dayjs().format('YYYY-MM-DD'))

const dayTitle = computed(() => {
  const d = dayjs(date.value)
  return `${d.month() + 1}月${d.date()}日 星期${WEEKDAYS[d.day()]}`
})

const pendingTasks = computed(() => {
  const all = tasks.value.filter((t) => !t.done)
  if (quadrantFilter.value === 'all') return all
  return all.filter((t) => quadrantOf(t) === quadrantFilter.value)
})
const doneTasks = computed(() => tasks.value.filter((t) => t.done))
const doneCount = computed(() => doneTasks.value.length)
const derivedCount = computed(() => tasks.value.filter((t) => isDerived(t)).length)
const percent = computed(() =>
  tasks.value.length === 0 ? 0 : Math.round((doneCount.value / tasks.value.length) * 100)
)

function quadrantOf(t) {
  if (t.important && t.urgent) return 'q1'
  if (t.important) return 'q2'
  if (t.urgent) return 'q3'
  return 'q4'
}

function isDerived(t) {
  return typeof t.id === 'string' && t.id.startsWith('proj:')
}

function evTagClass(tag) {
  const map = { 工作: 'work', 运动: 'sport', 学习: 'study', 看书: 'study', 生活: 'life' }
  return map[tag] || 'life'
}
function evTagType(tag) {
  const map = { 工作: 'danger', 运动: 'success', 学习: 'warning', 看书: 'warning', 生活: 'info' }
  return map[tag] || 'info'
}
function fmtTime(ts) {
  return dayjs.unix(ts).format('HH:mm')
}

async function rolloverYesterday() {
  if (rolledOverToday.value) return
  const yesterday = dayjs().subtract(1, 'day').format('YYYY-MM-DD')
  try {
    const res = await api.rolloverTasks(yesterday)
    if (res.count > 0) {
      ElMessage.success(`已将 ${res.count} 个未完成任务移到今天`)
    }
  } catch (e) {
    console.warn('自动移任务失败:', e.message)
  } finally {
    rolledOverToday.value = true
  }
}

async function manualRollover() {
  rolling.value = true
  try {
    const yesterday = dayjs().subtract(1, 'day').format('YYYY-MM-DD')
    const res = await api.rolloverTasks(yesterday)
    if (res.count > 0) {
      ElMessage.success(`已将 ${res.count} 个未完成任务移到今天`)
    } else {
      ElMessage.info('昨天没有未完成任务')
    }
    await load()
  } catch (e) {
    ElMessage.error('移入失败: ' + e.message)
  } finally {
    rolling.value = false
  }
}

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
    await api.createTask({
      title,
      priority: newPriority.value,
      important: newImportant.value,
      urgent: newUrgent.value,
      estimated_minutes: newEstimate.value,
      date: date.value,
    })
    newTitle.value = ''
    newImportant.value = false
    newUrgent.value = false
    newEstimate.value = null
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adding.value = false
  }
}

async function toggleDone(task, done) {
  if (isDerived(task)) return
  try {
    await api.updateTask(task.id, { done })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

async function startPomodoro(task) {
  try {
    const res = await api.pomodoroTask(task.id)
    ElMessage.success(`🍅 番茄钟已启动（第 ${res.pomodoro_count} 个）`)
    await load()
    await loadOngoing()
  } catch (e) {
    if (e.status === 409) {
      ElMessage.warning('已有进行中事件，请先结束')
    } else {
      ElMessage.error(e.message)
    }
  }
}

async function startTiming(task) {
  try {
    await api.startEvent({ content: task.title })
    ElMessage.success('已开始计时')
    await loadOngoing()
  } catch (e) {
    if (e.status === 409) {
      ElMessage.warning('已有进行中的事件')
    } else {
      ElMessage.error(e.message)
    }
  }
}

async function loadOngoing() {
  try {
    const ev = await api.getOngoingEvent()
    ongoingEvent.value = ev
    if (ev) {
      updateOngoingDuration()
      if (!ongoingTimer) {
        ongoingTimer = setInterval(updateOngoingDuration, 1000)
      }
    } else {
      if (ongoingTimer) {
        clearInterval(ongoingTimer)
        ongoingTimer = null
      }
      ongoingDuration.value = ''
    }
  } catch {
    ongoingEvent.value = null
  }
}

function updateOngoingDuration() {
  if (!ongoingEvent.value) return
  const elapsed = Math.floor(Date.now() / 1000) - ongoingEvent.value.start_ts
  const h = Math.floor(elapsed / 3600)
  const m = Math.floor((elapsed % 3600) / 60)
  const s = elapsed % 60
  ongoingDuration.value = h > 0
    ? `${h}h${String(m).padStart(2, '0')}m`
    : `${m}m${String(s).padStart(2, '0')}s`
}

function toggleExpand(task) {
  if (isDerived(task)) return
  if (expandedTask.value === task.id) {
    expandedTask.value = null
  } else {
    expandedTask.value = task.id
    editNotes.value = task.notes || ''
  }
}

async function saveNotes(task) {
  try {
    await api.updateTask(task.id, { notes: editNotes.value })
  } catch (e) {
    ElMessage.error('保存备注失败: ' + e.message)
  }
}

function openEdit(task) {
  editForm.id = task.id
  editForm.title = task.title
  editForm.priority = PRIORITIES.includes(task.priority) ? task.priority : '中'
  editForm.important = !!task.important
  editForm.urgent = !!task.urgent
  editForm.estimated_minutes = task.estimated_minutes || null
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
      important: editForm.important,
      urgent: editForm.urgent,
      estimated_minutes: editForm.estimated_minutes,
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

async function loadTimeline() {
  if (!isToday.value) return
  try {
    timeline.value = await api.getEvents(date.value)
  } catch {
    // 静默
  }
}

async function addExpense() {
  if (!expenseItem.value.trim()) {
    ElMessage.warning('输入买了什么')
    return
  }
  if (expenseAmount.value <= 0) {
    ElMessage.warning('输入金额')
    return
  }
  expenseAdding.value = true
  try {
    await api.createExpense({
      item: expenseItem.value.trim(),
      amount: expenseAmount.value,
      category: expenseCategory.value,
    })
    expenseItem.value = ''
    expenseAmount.value = 0
    ElMessage.success('已记账')
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    expenseAdding.value = false
  }
}

onMounted(async () => {
  await rolloverYesterday()
  load()
  loadOngoing()
  loadTimeline()
})

onUnmounted(() => {
  if (ongoingTimer) {
    clearInterval(ongoingTimer)
  }
})
</script>

<style scoped>
.head-card { margin-bottom: 14px; }
.head-row { display: flex; justify-content: space-between; align-items: center; }
.date-nav { display: flex; align-items: center; gap: 12px; }
.date-title { font-size: 18px; font-weight: 600; }
.progress-box { display: flex; align-items: center; gap: 12px; }
.progress-text { font-size: 13px; color: #909399; }

.add-card { margin-bottom: 14px; }
.add-row { display: flex; gap: 10px; }

.filter-card { margin-bottom: 14px; display: flex; align-items: center; }

.task-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 4px;
  border-bottom: 1px solid #f0f0f0;
  flex-wrap: wrap;
}
.task-row:hover { background: #f5f6fa; }
.task-row.done { opacity: 0.5; }
.task-row.done .task-title { text-decoration: line-through; }
.task-row.derived { background: #fdf6ec; border-radius: 4px; }
.task-row.expanded { background: #f0f9ff; }

.task-title { flex: 1; cursor: pointer; }
.task-ops { display: none; gap: 6px; }
.task-row:hover .task-ops { display: inline-flex; }
.op-danger { color: #f56c6c; }

.task-expand {
  width: 100%;
  margin-top: 6px;
  padding-left: 28px;
}

.done-section { margin-top: 10px; border-top: 1px dashed #e4e7ed; padding-top: 8px; }
.done-header { display: flex; align-items: center; gap: 6px; cursor: pointer; font-size: 13px; color: #909399; padding: 4px 0; }
.collapse-icon { transition: transform 0.2s; }
.collapse-icon.collapsed { transform: rotate(-90deg); }

.timeline-card { margin-top: 14px; }
.timeline-empty { color: #909399; font-size: 13px; padding: 12px 0; text-align: center; }
.timeline { padding-left: 8px; }
.timeline-item { display: flex; gap: 10px; padding: 6px 0; border-left: 2px solid #e4e7ed; margin-left: 6px; padding-left: 14px; }
.timeline-dot { width: 10px; height: 10px; border-radius: 50%; flex-shrink: 0; margin-top: 4px; margin-left: -19px; }
.timeline-dot.work { background: #f56c6c; }
.timeline-dot.sport { background: #67c23a; }
.timeline-dot.study { background: #e6a23c; }
.timeline-dot.life { background: #909399; }
.timeline-body { flex: 1; }
.timeline-time { font-size: 12px; color: #909399; }
.timeline-content { font-size: 14px; }

.expense-card { margin-top: 14px; }
.expense-row { display: flex; gap: 10px; align-items: center; }
</style>
