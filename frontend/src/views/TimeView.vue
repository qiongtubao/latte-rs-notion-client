<template>
  <div>
    <!-- 当前进行中事件 -->
    <el-card class="now-card" shadow="never">
      <div v-if="current" class="now-running">
        <div>
          <div class="now-label">进行中 · 开始于 {{ fmtTs(current.start_ts) }}</div>
          <div class="now-timer">{{ elapsed }}</div>
        </div>
        <el-button type="danger" size="large" @click="stopDialog = true">结束</el-button>
      </div>
      <div v-else class="now-idle">
        <el-button type="success" size="large" class="start-btn" :loading="starting" @click="start">
          开始
        </el-button>
        <div class="idle-hint">点击开始记录一段时间碎片</div>
      </div>
    </el-card>

    <!-- 结束事件对话框 -->
    <el-dialog v-model="stopDialog" title="结束当前事件" width="420px">
      <el-form label-width="80px">
        <el-form-item label="做了什么">
          <el-input v-model="stopForm.content" type="textarea" :rows="2" placeholder="这段时间做了什么？" />
        </el-form-item>
        <el-form-item label="标签">
          <el-radio-group v-model="stopForm.tag">
            <el-radio-button v-for="t in EVENT_TAGS" :key="t" :value="t">
              <el-tag :type="tagType(t)" size="small">{{ t }}</el-tag>
            </el-radio-button>
          </el-radio-group>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="stopDialog = false">取消</el-button>
        <el-button type="primary" :loading="stopping" @click="stop">确定</el-button>
      </template>
    </el-dialog>

    <!-- 今日事件 -->
    <el-card shadow="never" class="section">
      <template #header>
        <div class="section-header">
          <span>今日事件（{{ today }}）</span>
          <el-button text type="primary" @click="loadEvents">刷新</el-button>
        </div>
      </template>
      <el-table :data="events" empty-text="今天还没有记录" v-loading="loadingEvents">
        <el-table-column label="开始" width="70">
          <template #default="{ row }">{{ fmtTime(row.start_ts) }}</template>
        </el-table-column>
        <el-table-column label="结束" width="70">
          <template #default="{ row }">{{ row.end_ts ? fmtTime(row.end_ts) : '进行中' }}</template>
        </el-table-column>
        <el-table-column label="时长" width="90">
          <template #default="{ row }">{{ fmtDuration(row.duration_seconds) }}</template>
        </el-table-column>
        <el-table-column prop="content" label="内容" show-overflow-tooltip />
        <el-table-column label="标签" width="90">
          <template #default="{ row }">
            <el-tag v-if="row.tag" :type="tagType(row.tag)" size="small">{{ row.tag }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="操作" width="110">
          <template #default="{ row }">
            <el-button text type="primary" size="small" @click="openEdit(row)">编辑</el-button>
            <el-button text type="danger" size="small" @click="removeEvent(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 编辑事件对话框 -->
    <el-dialog v-model="editDialog" title="编辑事件" width="480px">
      <el-form label-width="80px">
        <el-form-item label="内容">
          <el-input v-model="editForm.content" type="textarea" :rows="2" />
        </el-form-item>
        <el-form-item label="标签">
          <el-select v-model="editForm.tag" style="width: 100%">
            <el-option v-for="t in EVENT_TAGS" :key="t" :label="t" :value="t" />
          </el-select>
        </el-form-item>
        <el-form-item label="开始时间">
          <el-date-picker v-model="editForm.start" type="datetime" style="width: 100%" value-format="X" />
        </el-form-item>
        <el-form-item label="结束时间">
          <el-date-picker v-model="editForm.end" type="datetime" style="width: 100%" value-format="X" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editDialog = false">取消</el-button>
        <el-button type="primary" :loading="saving" @click="saveEdit">保存</el-button>
      </template>
    </el-dialog>

    <!-- 报表 -->
    <el-card shadow="never" class="section">
      <template #header>
        <div class="section-header">
          <span>时间报表</span>
          <div class="report-controls">
            <el-radio-group v-model="period" size="small" @change="loadReport">
              <el-radio-button value="day">日</el-radio-button>
              <el-radio-button value="week">周</el-radio-button>
              <el-radio-button value="month">月</el-radio-button>
              <el-radio-button value="year">年</el-radio-button>
            </el-radio-group>
            <el-button-group>
              <el-button size="small" @click="shiftReport(-1)">←</el-button>
              <el-button size="small" @click="reportDate = today; loadReport()">今天</el-button>
              <el-button size="small" @click="shiftReport(1)">→</el-button>
            </el-button-group>
          </div>
        </div>
      </template>
      <div class="report-range">{{ report.range_start }} ~ {{ report.range_end }}</div>
      <div class="report-total">总时长：<b>{{ fmtDuration(report.total_seconds) }}</b></div>
      <div v-for="item in report.by_tag" :key="item.tag" class="report-row">
        <el-tag :type="tagType(item.tag)" size="small" class="report-tag">{{ item.tag }}</el-tag>
        <el-progress :percentage="item.percent" :stroke-width="14" class="report-bar" />
        <span class="report-seconds">{{ fmtDuration(item.seconds) }}</span>
      </div>
      <el-empty v-if="!report.by_tag || report.by_tag.length === 0" description="该时间段没有数据" :image-size="60" />
    </el-card>
  </div>
</template>

<script setup>
import { computed, inject, nextTick, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from '../api'
import { EVENT_TAGS, fmtDuration, fmtHMS, fmtTime, fmtTs, tagType } from '../utils'

const today = dayjs().format('YYYY-MM-DD')
const events = ref([])
const loadingEvents = ref(false)

const current = computed(() => events.value.find((e) => e.end_ts === null || e.end_ts === undefined))

const now = ref(dayjs().unix())
const elapsed = computed(() => (current.value ? fmtHMS(now.value - current.value.start_ts) : ''))
let tickTimer = null

const starting = ref(false)
const stopDialog = ref(false)
const stopping = ref(false)
const stopForm = reactive({ content: '', tag: '工作' })

const editDialog = ref(false)
const saving = ref(false)
const editForm = reactive({ id: null, content: '', tag: '工作', start: null, end: null })

const period = ref('day')
const reportDate = ref(today)
const report = ref({ range_start: '', range_end: '', total_seconds: 0, by_tag: [] })

async function loadEvents() {
  loadingEvents.value = true
  try {
    events.value = await api.getEvents(today)
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loadingEvents.value = false
  }
}

async function start() {
  starting.value = true
  try {
    await api.startEvent()
    await loadEvents()
  } catch (e) {
    ElMessage.error(e.status === 409 ? '已有进行中的事件' : e.message)
    await loadEvents()
  } finally {
    starting.value = false
  }
}

async function stop() {
  if (!stopForm.content.trim()) {
    ElMessage.warning('请填写「做了什么」')
    return
  }
  stopping.value = true
  try {
    await api.stopEvent(current.value.id, stopForm.content.trim(), stopForm.tag)
    stopDialog.value = false
    stopForm.content = ''
    await Promise.all([loadEvents(), loadReport()])
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    stopping.value = false
  }
}

function openEdit(row) {
  editForm.id = row.id
  editForm.content = row.content
  editForm.tag = row.tag || '工作'
  editForm.start = String(row.start_ts)
  editForm.end = row.end_ts ? String(row.end_ts) : null
  editDialog.value = true
}

async function saveEdit() {
  saving.value = true
  try {
    const data = { content: editForm.content, tag: editForm.tag }
    if (editForm.start) data.start_ts = Number(editForm.start)
    if (editForm.end) data.end_ts = Number(editForm.end)
    await api.updateEvent(editForm.id, data)
    editDialog.value = false
    await Promise.all([loadEvents(), loadReport()])
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}

async function removeEvent(row) {
  try {
    await ElMessageBox.confirm(`删除事件「${row.content || '(无内容)'}」？`, '确认删除', { type: 'warning' })
  } catch {
    return
  }
  try {
    await api.deleteEvent(row.id)
    await Promise.all([loadEvents(), loadReport()])
  } catch (e) {
    ElMessage.error(e.message)
  }
}

async function loadReport() {
  try {
    report.value = await api.getTimeReport(period.value, reportDate.value)
  } catch (e) {
    ElMessage.error(e.message)
  }
}

function shiftReport(delta) {
  const unit = { day: 'day', week: 'week', month: 'month', year: 'year' }[period.value]
  reportDate.value = dayjs(reportDate.value).add(delta, unit).format('YYYY-MM-DD')
  loadReport()
}
const subAction = inject('subAction')
watch(subAction, (act) => {
  if (!act || act.key !== 'time') return
  if (act.action === 'start') {
    if (current.value) {
      ElMessage.warning('已有进行中的事件')
    } else {
      start()
    }
  } else if (act.action === 'report') {
    nextTick(() => {
      const cards = document.querySelectorAll('.panel-body .el-card')
      const report = cards[cards.length - 1]
      if (report) report.scrollIntoView({ behavior: 'smooth', block: 'start' })
    })
  }
})

onMounted(() => {
  loadEvents()
  loadReport()
  tickTimer = setInterval(() => {
    now.value = dayjs().unix()
  }, 1000)
})

onUnmounted(() => clearInterval(tickTimer))
</script>

<style scoped>
.now-card {
  text-align: center;
}

.now-running {
  display: flex;
  justify-content: space-between;
  align-items: center;
  text-align: left;
}

.now-label {
  color: #909399;
  font-size: 13px;
}

.now-timer {
  font-size: 42px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
  color: #409eff;
}

.now-idle {
  padding: 12px 0;
}

.start-btn {
  width: 140px;
  height: 56px;
  font-size: 20px;
}

.idle-hint {
  margin-top: 10px;
  color: #909399;
  font-size: 13px;
}

.section {
  margin-top: 16px;
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.report-controls {
  display: flex;
  gap: 10px;
  align-items: center;
}

.report-range {
  color: #909399;
  font-size: 13px;
  margin-bottom: 6px;
}

.report-total {
  margin-bottom: 14px;
}

.report-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 8px;
}

.report-tag {
  width: 48px;
  text-align: center;
}

.report-bar {
  flex: 1;
}

.report-seconds {
  width: 70px;
  text-align: right;
  font-size: 13px;
  color: #606266;
}
</style>
