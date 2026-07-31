<template>
  <div>
    <el-card shadow="never">
      <template #header>
        <div class="cal-header">
          <el-button-group>
            <el-button @click="shiftMonth(-1)">←</el-button>
            <el-button @click="goToday">今天</el-button>
            <el-button @click="shiftMonth(1)">→</el-button>
          </el-button-group>
          <span class="cal-title">{{ monthLabel }}</span>
        </div>
      </template>

      <div class="grid" v-loading="loading">
        <div v-for="w in weekNames" :key="w" class="cell week-name">{{ w }}</div>
        <div
          v-for="cell in cells"
          :key="cell.key"
          class="cell day"
          :class="{ dim: !cell.inMonth, today: cell.isToday, clickable: cell.inMonth }"
          @click="cell.inMonth && openDay(cell.date)"
        >
          <div class="day-num">{{ cell.dayNum }}</div>
          <div v-if="cell.data && cell.data.seconds > 0" class="day-info time">
            ⏱ {{ fmtHours(cell.data.seconds) }}
          </div>
          <div v-if="cell.data && cell.data.expense > 0" class="day-info expense">
            {{ fmtMoney(cell.data.expense) }}
          </div>
        </div>
      </div>
    </el-card>

    <!-- 当日明细抽屉：24 小时时间线 + 列表 -->
    <el-drawer v-model="drawer" size="580px">
      <template #header>
        <div class="drawer-header">
          <el-button-group>
            <el-button size="small" @click="shiftDay(-1)">←</el-button>
            <el-button size="small" @click="openDay(todayStr)">今天</el-button>
            <el-button size="small" @click="shiftDay(1)">→</el-button>
          </el-button-group>
          <span class="drawer-title">{{ selectedDate }} {{ weekdayLabel }}</span>
          <el-button size="small" class="ai-btn" @click="openAi">图片识别</el-button>
        </div>
      </template>

      <div v-loading="dayLoading">
        <!-- 24 小时时间线 -->
        <template v-if="blocks.length > 0">
          <div class="timeline-scroll" ref="timelineRef">
            <div class="timeline" :style="{ height: TOTAL_H + 'px' }">
              <div class="hours">
                <div v-for="h in 24" :key="h" class="hour-row">
                  <span class="hour-label">{{ String(h - 1).padStart(2, '0') }}:00</span>
                </div>
              </div>
              <div
                class="track"
                @click.self="onTrackClick"
                @mousemove="onTrackHover"
                @mouseleave="hoverTop = null"
              >
                <div
                  v-if="hoverTop !== null"
                  class="hover-band"
                  :style="{ top: hoverTop + 'px' }"
                />
                <el-tooltip
                  v-for="b in blocks"
                  :key="b.event.id"
                  placement="right"
                  :content="`${b.event.remind ? '🔔 ' : ''}${b.event.tag || '未分类'} · ${b.event.content || '(无内容)'} · ${b.rangeText}`"
                >
                  <div
                    class="block"
                    :class="{ ongoing: b.ongoing, thin: b.thin }"
                    :style="{ top: b.top + 'px', height: b.height + 'px', background: b.color }"
                  >
                    <template v-if="!b.thin">
                      <div class="block-line1">
                        <span v-if="b.event.remind" class="block-bell">🔔</span>
                        <b>{{ b.event.tag || '未分类' }}</b> {{ b.event.content }}
                      </div>
                      <div v-if="b.height >= 56" class="block-line2">{{ b.rangeText }}</div>
                    </template>
                  </div>
                </el-tooltip>
                <div v-if="isToday" class="now-line" :style="{ top: nowTop + 'px' }" />
              </div>
            </div>
          </div>
        </template>
        <el-empty v-else description="这一天没有时间碎片" :image-size="60" />

        <!-- 合计 -->
        <div class="totals">
          <span>总时长：<b>{{ fmtDuration(totalSeconds) }}</b></span>
          <span>总消费：<b>{{ fmtCents(totalCents) }}</b></span>
        </div>

        <!-- 事件明细 -->
        <h4>时间碎片</h4>
        <el-table :data="dayDetail.events" empty-text="无记录" size="small">
          <el-table-column label="开始" width="60">
            <template #default="{ row }">{{ fmtTime(row.start_ts) }}</template>
          </el-table-column>
          <el-table-column label="结束" width="60">
            <template #default="{ row }">{{ row.end_ts ? fmtTime(row.end_ts) : '进行中' }}</template>
          </el-table-column>
          <el-table-column label="时长" width="80">
            <template #default="{ row }">{{ fmtDuration(row.duration_seconds) }}</template>
          </el-table-column>
          <el-table-column prop="content" label="内容" show-overflow-tooltip />
          <el-table-column label="标签" width="70">
            <template #default="{ row }">
              <el-tag v-if="row.tag" :type="tagType(row.tag)" size="small">{{ row.tag }}</el-tag>
            </template>
          </el-table-column>
        </el-table>

        <h4 class="mt">消费</h4>
        <el-table :data="dayDetail.expenses" empty-text="无记录" size="small">
          <el-table-column label="时间" width="60">
            <template #default="{ row }">{{ fmtTime(row.ts) }}</template>
          </el-table-column>
          <el-table-column prop="item" label="事项" show-overflow-tooltip />
          <el-table-column prop="category" label="分类" width="70" />
          <el-table-column label="金额" width="90" align="right">
            <template #default="{ row }">{{ fmtCents(row.amount_cents) }}</template>
          </el-table-column>
        </el-table>
      </div>
    </el-drawer>

    <!-- 点击时间线创建事件 -->
    <el-dialog v-model="createDialog" title="新建事件" width="460px">
      <el-form label-width="80px">
        <el-form-item label="开始时间">
          <el-date-picker v-model="createForm.start_ts" type="datetime" style="width: 100%" value-format="X" />
        </el-form-item>
        <el-form-item label="结束时间">
          <el-date-picker v-model="createForm.end_ts" type="datetime" style="width: 100%" value-format="X" />
        </el-form-item>
        <el-form-item label="内容">
          <el-input v-model="createForm.content" placeholder="这段时间做了什么？" />
        </el-form-item>
        <el-form-item label="标签">
          <el-select v-model="createForm.tag" style="width: 100%">
            <el-option v-for="t in EVENT_TAGS" :key="t" :label="t" :value="t" />
          </el-select>
        </el-form-item>
        <el-form-item label="到点提醒">
          <el-checkbox v-model="createForm.remind">开始时发送浏览器通知</el-checkbox>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createDialog = false">取消</el-button>
        <el-button type="primary" :loading="creating" @click="saveCreate">保存</el-button>
      </template>
    </el-dialog>

    <!-- 图片识别上传 -->
    <el-dialog v-model="aiDialog" title="图片识别添加事件" width="480px">
      <el-upload
        drag
        accept="image/*"
        :auto-upload="false"
        :limit="1"
        :on-change="onFilePick"
        :on-remove="clearImage"
        :file-list="fileList"
      >
        <div class="upload-hint">拖拽图片到这里，或 <em>点击选择</em></div>
      </el-upload>
      <div v-if="previewUrl" class="preview-wrap">
        <img :src="previewUrl" class="preview-img" alt="预览" />
      </div>
      <template #footer>
        <el-button @click="aiDialog = false">取消</el-button>
        <el-button type="primary" :disabled="!imageBase64" :loading="recognizing" @click="recognize">
          {{ recognizing ? '识别中…' : '开始识别' }}
        </el-button>
      </template>
    </el-dialog>

    <!-- 识别结果编辑 -->
    <el-dialog v-model="sugDialog" title="识别结果（勾选后添加）" width="720px">
      <el-table :data="suggestions" size="small">
        <el-table-column width="40">
          <template #default="{ row }">
            <el-checkbox v-model="row.checked" />
          </template>
        </el-table-column>
        <el-table-column label="开始" width="185">
          <template #default="{ row }">
            <el-date-picker v-model="row.start_ts" type="datetime" size="small" style="width: 170px" value-format="X" />
          </template>
        </el-table-column>
        <el-table-column label="结束" width="185">
          <template #default="{ row }">
            <el-date-picker v-model="row.end_ts" type="datetime" size="small" style="width: 170px" value-format="X" />
          </template>
        </el-table-column>
        <el-table-column label="内容" min-width="120">
          <template #default="{ row }">
            <el-input v-model="row.content" size="small" />
          </template>
        </el-table-column>
        <el-table-column label="标签" width="100">
          <template #default="{ row }">
            <el-select v-model="row.tag" size="small">
              <el-option v-for="t in EVENT_TAGS" :key="t" :label="t" :value="t" />
            </el-select>
          </template>
        </el-table-column>
        <el-table-column width="60">
          <template #default="{ $index }">
            <el-button text type="danger" size="small" @click="suggestions.splice($index, 1)">删行</el-button>
          </template>
        </el-table-column>
      </el-table>
      <template #footer>
        <el-button @click="sugDialog = false">取消</el-button>
        <el-button type="primary" :loading="adding" @click="addSelected">添加所选</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, nextTick, onMounted, onUnmounted, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'
import { EVENT_TAGS, TAG_COLORS, fmtCents, fmtDuration, fmtHours, fmtMoney, fmtTime, tagType } from '../utils'

const weekNames = ['一', '二', '三', '四', '五', '六', '日']
const month = ref(dayjs().format('YYYY-MM'))
const dataMap = ref({})
const loading = ref(false)

const drawer = ref(false)
const dayLoading = ref(false)
const selectedDate = ref('')
const dayDetail = ref({ events: [], expenses: [] })

const todayStr = dayjs().format('YYYY-MM-DD')

// 时间线常量：每小时 48px
const HOUR_H = 48
const TOTAL_H = HOUR_H * 24

const nowTs = ref(dayjs().unix())
let nowTimer = null

const timelineRef = ref(null)

const monthLabel = computed(() => dayjs(month.value + '-01').format('YYYY 年 M 月'))

const isToday = computed(() => selectedDate.value === todayStr)

const weekdayLabel = computed(() =>
  selectedDate.value ? '周' + weekNames[(dayjs(selectedDate.value).day() + 6) % 7] : ''
)

const dayBounds = computed(() => {
  const start = dayjs(selectedDate.value).startOf('day').unix()
  return { start, end: start + 86400 }
})

// 事件裁剪到当天 0:00–24:00 后的色块布局
const blocks = computed(() => {
  const { start, end } = dayBounds.value
  const result = []
  for (const ev of dayDetail.value.events || []) {
    const s = Math.max(ev.start_ts, start)
    const e = Math.min(ev.end_ts ?? nowTs.value, end)
    if (e <= s) continue
    const top = ((s - start) / 86400) * TOTAL_H
    const height = Math.max(((e - s) / 86400) * TOTAL_H, 3)
    const ongoing = ev.end_ts === null || ev.end_ts === undefined
    result.push({
      event: ev,
      top,
      height,
      ongoing,
      thin: e - s < 15 * 60,
      color: TAG_COLORS[ev.tag] || '#909399',
      rangeText: `${fmtTime(s)}–${ongoing ? '进行中' : fmtTime(e)}`,
    })
  }
  return result.sort((a, b) => a.top - b.top)
})

const totalSeconds = computed(() =>
  blocks.value.reduce((sum, b) => sum + Math.round((b.height / TOTAL_H) * 86400), 0)
)

const totalCents = computed(() =>
  (dayDetail.value.expenses || []).reduce((sum, e) => sum + (e.amount_cents || 0), 0)
)

// 当前时刻指示线位置（仅查看今天时渲染）
const nowTop = computed(() => {
  const { start } = dayBounds.value
  return ((nowTs.value - start) / 86400) * TOTAL_H
})

// ---------- 时间线点击创建 ----------

const hoverTop = ref(null)
const createDialog = ref(false)
const creating = ref(false)
const createForm = reactive({ start_ts: null, end_ts: null, content: '', tag: '工作', remind: false })

// y 坐标 → 当天分钟数（吸附 15 分钟）
function yToSnappedMinutes(y) {
  const minutes = (y / TOTAL_H) * 1440
  return Math.min(Math.max(Math.round(minutes / 15) * 15, 0), 1440 - 15)
}

function onTrackHover(e) {
  if (e.target !== e.currentTarget) {
    hoverTop.value = null
    return
  }
  const rect = e.currentTarget.getBoundingClientRect()
  const snapped = yToSnappedMinutes(e.clientY - rect.top)
  hoverTop.value = (snapped / 1440) * TOTAL_H
}

function onTrackClick(e) {
  const rect = e.currentTarget.getBoundingClientRect()
  const snapped = yToSnappedMinutes(e.clientY - rect.top)
  const startTs = dayBounds.value.start + snapped * 60
  createForm.start_ts = String(startTs)
  createForm.end_ts = String(startTs + 3600)
  createForm.content = ''
  createForm.tag = '工作'
  createForm.remind = false
  createDialog.value = true
}

async function saveCreate() {
  if (!createForm.content.trim()) {
    ElMessage.warning('请填写内容')
    return
  }
  if (!createForm.start_ts || !createForm.end_ts) {
    ElMessage.warning('请选择开始和结束时间')
    return
  }
  creating.value = true
  try {
    await api.createEvent({
      start_ts: Number(createForm.start_ts),
      end_ts: Number(createForm.end_ts),
      content: createForm.content.trim(),
      tag: createForm.tag,
      remind: createForm.remind,
    })
    createDialog.value = false
    await reloadDay()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    creating.value = false
  }
}

// ---------- 图片 AI 识别 ----------

const aiDialog = ref(false)
const recognizing = ref(false)
const imageBase64 = ref('')
const mediaType = ref('')
const previewUrl = ref('')
const fileList = ref([])

const sugDialog = ref(false)
const adding = ref(false)
const suggestions = ref([])

function openAi() {
  aiDialog.value = true
}

function onFilePick(uploadFile) {
  const raw = uploadFile.raw
  if (!raw) return
  fileList.value = [uploadFile]
  mediaType.value = raw.type || 'image/png'
  const reader = new FileReader()
  reader.onload = () => {
    const dataUrl = reader.result
    previewUrl.value = dataUrl
    imageBase64.value = String(dataUrl).split(',')[1] || ''
  }
  reader.readAsDataURL(raw)
}

function clearImage() {
  fileList.value = []
  imageBase64.value = ''
  mediaType.value = ''
  previewUrl.value = ''
}

async function recognize() {
  recognizing.value = true
  try {
    const r = await api.recognizeEvents(imageBase64.value, mediaType.value, selectedDate.value)
    const list = r.suggestions || []
    if (list.length === 0) {
      ElMessage.info('未识别到事件')
      return
    }
    suggestions.value = list.map((s) => ({
      checked: true,
      start_ts: String(s.start_ts),
      end_ts: String(s.end_ts),
      content: s.content || '',
      tag: EVENT_TAGS.includes(s.tag) ? s.tag : '生活',
    }))
    aiDialog.value = false
    sugDialog.value = true
  } catch (e) {
    if (e.status === 502) {
      ElMessage.error(`${e.message}（请检查 latte-model-proxy 是否启动）`)
    } else {
      ElMessage.error(e.message)
    }
  } finally {
    recognizing.value = false
  }
}

async function addSelected() {
  const selected = suggestions.value.filter((s) => s.checked)
  if (selected.length === 0) {
    ElMessage.warning('请至少勾选一条')
    return
  }
  adding.value = true
  try {
    await Promise.all(
      selected.map((s) =>
        api.createEvent({
          start_ts: Number(s.start_ts),
          end_ts: Number(s.end_ts),
          content: s.content,
          tag: s.tag,
          remind: false,
        })
      )
    )
    ElMessage.success(`已添加 ${selected.length} 条`)
    sugDialog.value = false
    clearImage()
    await reloadDay()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adding.value = false
  }
}

// ---------- 月历网格 ----------

// 构造 7 列网格，周一开头；不足处用相邻月份补齐
const cells = computed(() => {
  const first = dayjs(month.value + '-01')
  const daysInMonth = first.daysInMonth()
  // dayjs: 0=周日..6=周六 → 周一开头的偏移
  const offset = (first.day() + 6) % 7
  const result = []

  for (let i = 0; i < offset; i++) {
    const d = first.subtract(offset - i, 'day')
    result.push({ key: 'p' + i, date: d.format('YYYY-MM-DD'), dayNum: d.date(), inMonth: false })
  }
  for (let d = 1; d <= daysInMonth; d++) {
    const date = first.date(d).format('YYYY-MM-DD')
    result.push({
      key: date,
      date,
      dayNum: d,
      inMonth: true,
      isToday: date === todayStr,
      data: dataMap.value[date],
    })
  }
  const remainder = result.length % 7
  if (remainder > 0) {
    for (let i = 1; i <= 7 - remainder; i++) {
      const d = first.endOf('month').add(i, 'day')
      result.push({ key: 'n' + i, date: d.format('YYYY-MM-DD'), dayNum: d.date(), inMonth: false })
    }
  }
  return result
})

async function loadCalendar() {
  loading.value = true
  try {
    const list = await api.getCalendar(month.value)
    const map = {}
    for (const item of list) map[item.date] = item
    dataMap.value = map
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loading.value = false
  }
}

function shiftMonth(delta) {
  month.value = dayjs(month.value + '-01').add(delta, 'month').format('YYYY-MM')
  loadCalendar()
}

function goToday() {
  month.value = dayjs().format('YYYY-MM')
  loadCalendar()
}

async function reloadDay() {
  // 刷新当日明细 + 月历汇总（新建/识别添加后调用）
  dayDetail.value = await api.getCalendarDay(selectedDate.value)
  loadCalendar()
}

async function openDay(date) {
  selectedDate.value = date
  drawer.value = true
  dayLoading.value = true
  try {
    dayDetail.value = await api.getCalendarDay(date)
    await nextTick()
    scrollTimeline()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    dayLoading.value = false
  }
}

function shiftDay(delta) {
  openDay(dayjs(selectedDate.value).add(delta, 'day').format('YYYY-MM-DD'))
}

// 默认滚到当天第一个事件附近；无事件滚到 8:00
function scrollTimeline() {
  const el = timelineRef.value
  if (!el) return
  const target = blocks.value.length > 0 ? blocks.value[0].top - HOUR_H : 8 * HOUR_H
  el.scrollTop = Math.max(0, target)
}

onMounted(() => {
  loadCalendar()
  nowTimer = setInterval(() => {
    nowTs.value = dayjs().unix()
  }, 60000)
})

onUnmounted(() => clearInterval(nowTimer))
</script>

<style scoped>
.cal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.cal-title {
  font-size: 17px;
  font-weight: 600;
}

.grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
  gap: 4px;
}

.cell {
  min-height: 72px;
  border-radius: 6px;
  padding: 6px 8px;
}

.week-name {
  min-height: 0;
  text-align: center;
  color: #909399;
  font-size: 13px;
  padding: 4px 0;
}

.day {
  background: #fafafa;
  border: 1px solid #f0f0f0;
}

.day.clickable {
  cursor: pointer;
}

.day.clickable:hover {
  background: #ecf5ff;
  border-color: #b3d8ff;
}

.day.dim {
  opacity: 0.35;
}

.day.today {
  border-color: #409eff;
  background: #ecf5ff;
}

.day.today .day-num {
  color: #409eff;
  font-weight: 700;
}

.day-num {
  font-size: 14px;
}

.day-info {
  font-size: 12px;
  margin-top: 3px;
}

.day-info.time {
  color: #409eff;
}

.day-info.expense {
  color: #e6a23c;
}

/* ---------- 抽屉：24 小时时间线 ---------- */

.drawer-header {
  display: flex;
  align-items: center;
  gap: 12px;
}

.drawer-title {
  font-size: 16px;
  font-weight: 600;
}

.ai-btn {
  margin-left: auto;
}

.timeline-scroll {
  max-height: 420px;
  overflow-y: auto;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  margin-bottom: 14px;
}

.timeline {
  display: flex;
  position: relative;
}

.hours {
  flex: 0 0 52px;
  border-right: 1px solid #e4e7ed;
}

.hour-row {
  height: 48px;
  border-bottom: 1px solid #f0f0f0;
  position: relative;
}

.hour-label {
  position: absolute;
  top: 2px;
  left: 6px;
  font-size: 11px;
  color: #909399;
}

.track {
  flex: 1;
  position: relative;
  cursor: pointer;
  background: repeating-linear-gradient(
    to bottom,
    transparent 0,
    transparent 47px,
    #f0f0f0 47px,
    #f0f0f0 48px
  );
}

.hover-band {
  position: absolute;
  left: 0;
  right: 0;
  height: 12px;
  background: rgba(64, 158, 255, 0.18);
  pointer-events: none;
}

.block {
  position: absolute;
  left: 6px;
  right: 6px;
  border-radius: 6px;
  color: #fff;
  font-size: 12px;
  padding: 4px 8px;
  overflow: hidden;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.15);
}

.block.thin {
  padding: 0;
  border-radius: 2px;
}

.block.ongoing {
  border: 2px dashed rgba(255, 255, 255, 0.9);
  animation: pulse 1.6s ease-in-out infinite;
}

@keyframes pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.55;
  }
}

.block-line1 {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.block-bell {
  margin-right: 2px;
}

.block-line2 {
  opacity: 0.85;
  margin-top: 2px;
}

.now-line {
  position: absolute;
  left: 0;
  right: 0;
  height: 2px;
  background: #f56c6c;
  z-index: 2;
}

.now-line::before {
  content: '';
  position: absolute;
  left: 0;
  top: -3px;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #f56c6c;
}

.totals {
  display: flex;
  gap: 24px;
  color: #606266;
  font-size: 14px;
  margin-bottom: 6px;
}

.mt {
  margin-top: 24px;
}

/* ---------- 图片识别 ---------- */

.upload-hint {
  color: #909399;
  font-size: 13px;
  padding: 12px;
}

.preview-wrap {
  margin-top: 12px;
  text-align: center;
}

.preview-img {
  max-width: 100%;
  max-height: 200px;
  border-radius: 6px;
  border: 1px solid #e4e7ed;
}
</style>
