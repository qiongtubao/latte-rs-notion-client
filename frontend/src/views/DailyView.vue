<template>
  <div v-loading="loading">
    <!-- 今日打卡（习惯） -->
    <el-card shadow="never" class="mb">
      <template #header>
        <div class="card-head">
          <span>今日打卡 · {{ date }}</span>
        </div>
      </template>
      <el-empty v-if="habits.length === 0" description="还没有打卡习惯，下面添加一个" :image-size="60" />
      <div v-for="item in habits" :key="item.id" class="row">
        <el-checkbox :model-value="item.today_done" @change="(v) => toggleHabit(item, v)" />
        <span class="row-name" :class="{ done: item.today_done }">{{ item.name }}</span>
        <el-tag v-if="item.streak > 0" size="small" type="danger" effect="plain">🔥 {{ item.streak }} 天</el-tag>
        <el-icon class="row-op" title="归档" @click="archiveItem(item)"><Delete /></el-icon>
      </div>
      <div class="add-row">
        <el-input v-model="newHabit" size="small" placeholder="新习惯，如：早起 / 运动" @keyup.enter="addHabit" />
        <el-button size="small" type="primary" :disabled="!newHabit.trim()" @click="addHabit">添加</el-button>
      </div>
    </el-card>

    <!-- 数值记录（体重等） -->
    <el-card shadow="never" class="mb">
      <template #header>
        <div class="card-head"><span>数值记录</span></div>
      </template>
      <el-empty v-if="metrics.length === 0" description="还没有记录项，下面添加（如：体重 kg）" :image-size="60" />
      <div v-for="item in metrics" :key="item.id" class="row">
        <span class="row-name">{{ item.name }}<span v-if="item.unit" class="unit">（{{ item.unit }}）</span></span>
        <el-input
          v-model="metricInputs[item.id]"
          size="small"
          class="metric-input"
          :placeholder="item.last_value != null ? String(item.last_value) : '数值'"
          @keyup.enter="recordMetric(item)"
        />
        <el-button size="small" type="primary" :loading="recording === item.id" @click="recordMetric(item)">记录</el-button>
        <el-icon class="row-op" title="归档" @click="archiveItem(item)"><Delete /></el-icon>
      </div>
      <div class="add-row">
        <el-input v-model="newMetricName" size="small" placeholder="名称，如：体重" />
        <el-input v-model="newMetricUnit" size="small" placeholder="单位，如：kg" class="unit-input" />
        <el-button size="small" type="primary" :disabled="!newMetricName.trim()" @click="addMetric">添加</el-button>
      </div>
    </el-card>

    <!-- 近 14 天历史 -->
    <el-card shadow="never">
      <template #header>
        <div class="card-head"><span>近 14 天</span></div>
      </template>

      <!-- 习惯格子 -->
      <template v-if="habits.length > 0">
        <div class="grid-head" :style="gridStyle">
          <span class="grid-name"></span>
          <span v-for="d in dates" :key="d" class="grid-cell head">{{ dayLabel(d) }}</span>
        </div>
        <div v-for="item in habits" :key="item.id" class="grid-head" :style="gridStyle">
          <span class="grid-name" :title="item.name">{{ item.name }}</span>
          <span
            v-for="d in dates"
            :key="d"
            class="grid-cell"
            :class="{ on: habitDone(item.id, d), today: d === date }"
          >{{ habitDone(item.id, d) ? '✓' : '' }}</span>
        </div>
      </template>

      <!-- 数值最近记录 -->
      <div v-for="item in metricsWithHistory" :key="item.id" class="metric-hist">
        <span class="row-name">{{ item.name }}</span>
        <span class="metric-points">
          <span v-for="p in item.points" :key="p.date" class="metric-point">
            {{ shortDate(p.date) }} <b>{{ p.value }}{{ item.unit }}</b>
          </span>
          <span v-if="item.points.length === 0" class="no-data">暂无记录</span>
        </span>
      </div>
    </el-card>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Delete } from '@element-plus/icons-vue'
import { api } from '../api'

const HISTORY_DAYS = 14

const loading = ref(false)
const date = ref('')
const items = ref([])
const entries = ref([])
const recording = ref(null)

const newHabit = ref('')
const newMetricName = ref('')
const newMetricUnit = ref('')
const metricInputs = reactive({})

const habits = computed(() => items.value.filter((i) => i.kind === '打卡'))
const metrics = computed(() => items.value.filter((i) => i.kind === '记录'))

// 近 14 天日期列（升序，今天在最后）
const dates = computed(() => {
  const out = []
  for (let i = HISTORY_DAYS - 1; i >= 0; i--) {
    out.push(dayjs().subtract(i, 'day').format('YYYY-MM-DD'))
  }
  return out
})
const gridStyle = computed(() => ({ gridTemplateColumns: `72px repeat(${HISTORY_DAYS}, 1fr)` }))

// `${item_id}:${date}` -> entry
const entryMap = computed(() => {
  const m = {}
  for (const e of entries.value) m[`${e.item_id}:${e.date}`] = e
  return m
})

function habitDone(itemId, d) {
  return !!entryMap.value[`${itemId}:${d}`]?.done
}

const metricsWithHistory = computed(() =>
  metrics.value.map((item) => ({
    ...item,
    points: entries.value
      .filter((e) => e.item_id === item.id && e.value != null)
      .slice(-5),
  }))
)

function dayLabel(d) {
  return dayjs(d).format('D')
}
function shortDate(d) {
  return dayjs(d).format('MM-DD')
}

async function load() {
  loading.value = true
  try {
    const [overview, hist] = await Promise.all([api.getDaily(), api.getDailyHistory(HISTORY_DAYS)])
    date.value = overview.date
    items.value = overview.items
    entries.value = hist.entries
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loading.value = false
  }
}

async function toggleHabit(item, done) {
  try {
    await api.upsertDailyEntry({ item_id: item.id, done: !!done })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

async function recordMetric(item) {
  const raw = String(metricInputs[item.id] ?? '').trim()
  const value = parseFloat(raw)
  if (!raw || !Number.isFinite(value)) {
    ElMessage.warning('请输入数值')
    return
  }
  recording.value = item.id
  try {
    await api.upsertDailyEntry({ item_id: item.id, value })
    metricInputs[item.id] = ''
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    recording.value = null
  }
}

async function addHabit() {
  const name = newHabit.value.trim()
  if (!name) return
  try {
    await api.createDailyItem({ kind: '打卡', name })
    newHabit.value = ''
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

async function addMetric() {
  const name = newMetricName.value.trim()
  if (!name) return
  try {
    await api.createDailyItem({ kind: '记录', name, unit: newMetricUnit.value.trim() })
    newMetricName.value = ''
    newMetricUnit.value = ''
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

async function archiveItem(item) {
  try {
    await ElMessageBox.confirm(
      `归档「${item.name}」？归档后不再显示，历史记录保留。`,
      '归档打卡项',
      { confirmButtonText: '归档', cancelButtonText: '取消', type: 'warning' }
    )
  } catch { return }
  try {
    await api.updateDailyItem(item.id, { archived: true })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.mb { margin-bottom: 14px; }
.card-head { display: flex; justify-content: space-between; align-items: center; font-weight: 600; }
.row { display: flex; align-items: center; gap: 10px; padding: 6px 0; }
.row-name { flex: 1; font-size: 14px; color: #303133; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row-name.done { color: #909399; text-decoration: line-through; }
.unit { color: #909399; font-size: 12px; }
.row-op { color: #c0c4cc; cursor: pointer; flex-shrink: 0; }
.row-op:hover { color: #f56c6c; }
.add-row { display: flex; gap: 8px; margin-top: 10px; }
.metric-input { width: 110px; }
.unit-input { width: 80px; }
.grid-head { display: grid; gap: 2px; margin-bottom: 2px; align-items: center; }
.grid-name { font-size: 12px; color: #606266; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.grid-cell {
  height: 18px; line-height: 18px; text-align: center;
  font-size: 11px; color: #67c23a;
  background: #f5f7fa; border-radius: 3px;
}
.grid-cell.head { background: none; color: #909399; height: 16px; line-height: 16px; }
.grid-cell.on { background: #dcf5e3; font-weight: 700; }
.grid-cell.today { outline: 1px solid #409eff; }
.metric-hist { display: flex; align-items: baseline; gap: 10px; padding: 8px 0 2px; border-top: 1px dashed #ebeef5; margin-top: 8px; }
.metric-hist:first-of-type { border-top: none; margin-top: 0; }
.metric-points { display: flex; flex-wrap: wrap; gap: 10px; font-size: 12px; color: #606266; }
.no-data { color: #c0c4cc; }
</style>
