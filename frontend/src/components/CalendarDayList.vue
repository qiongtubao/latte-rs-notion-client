<template>
  <div class="cdl" v-loading="loading">
    <div class="cdl-header">
      <span class="cdl-title">日程</span>
      <span class="cdl-add" @click="showAdd = !showAdd">{{ showAdd ? '收起' : '＋ 添加' }}</span>
      <span class="cdl-open" @click="openFull">打开完整日历 →</span>
    </div>

    <!-- 添加事件（含到点提醒） -->
    <div v-if="showAdd" class="cdl-addform">
      <el-input
        v-model="form.content"
        size="small"
        placeholder="事件内容"
        @keyup.enter="saveEvent"
      />
      <el-date-picker
        v-model="form.range"
        type="datetimerange"
        size="small"
        range-separator="至"
        start-placeholder="开始"
        end-placeholder="结束"
        value-format="X"
        style="width: 100%"
      />
      <div class="cdl-addrow">
        <el-select v-model="form.tag" size="small" style="width: 84px">
          <el-option v-for="t in EVENT_TAGS" :key="t" :label="t" :value="t" />
        </el-select>
        <el-checkbox v-model="form.remind" size="small" title="开始时发送浏览器通知">🔔 提醒</el-checkbox>
        <el-button size="small" type="primary" :loading="saving" @click="saveEvent">保存</el-button>
      </div>
    </div>

    <div v-for="d in days" :key="d.date" class="cdl-day">
      <div class="cdl-date">{{ d.label }}</div>
      <div v-if="d.events.length === 0" class="cdl-empty">没有安排</div>
      <div
        v-for="e in d.events"
        :key="e.id"
        class="cdl-row"
        :style="{ borderLeftColor: tagColor(e.tag) }"
      >
        <span class="cdl-time">{{ fmtRange(e) }}</span>
        <span v-if="e.remind" class="cdl-bell" title="到点提醒">🔔</span>
        <span class="cdl-content" :title="e.content">{{ e.content || '(无内容)' }}</span>
        <span v-if="e.tag" class="cdl-tag" :style="{ background: tagColor(e.tag) }">{{ e.tag }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'
import { EVENT_TAGS } from '../utils'

const TAG_COLORS = {
  工作: '#f56c6c',
  运动: '#67c23a',
  学习: '#e6a23c',
  看书: '#e6a23c',
  生活: '#909399',
}
function tagColor(tag) {
  return TAG_COLORS[tag] || '#909399'
}

const days = ref([])
const loading = ref(false)

// 添加事件
const showAdd = ref(false)
const saving = ref(false)
const form = reactive({ content: '', range: null, tag: '工作', remind: false })

async function saveEvent() {
  if (!form.content.trim()) {
    ElMessage.warning('请填写内容')
    return
  }
  if (!form.range || !form.range[0] || !form.range[1]) {
    ElMessage.warning('请选择开始和结束时间')
    return
  }
  saving.value = true
  try {
    const remind = form.remind
    await api.createEvent({
      start_ts: Number(form.range[0]),
      end_ts: Number(form.range[1]),
      content: form.content.trim(),
      tag: form.tag,
      remind,
    })
    form.content = ''
    form.range = null
    form.remind = false
    ElMessage.success(remind ? '已添加，到点将发送通知' : '已添加')
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}

function fmtRange(e) {
  const start = dayjs.unix(e.start_ts).format('HH:mm')
  return e.end_ts ? `${start}-${dayjs.unix(e.end_ts).format('HH:mm')}` : `${start} 起`
}

// 打开完整日历面板（由 App.vue provide）
const openPanel = inject('openPanel', null)
function openFull() {
  if (openPanel) openPanel('calendar')
}

async function load() {
  loading.value = true
  try {
    const today = dayjs()
    const dates = [0, 1].map((i) => today.add(i, 'day'))
    const eventsList = await Promise.all(
      dates.map((d) => api.getEvents(d.format('YYYY-MM-DD')))
    )
    days.value = dates.map((d, i) => ({
      date: d.format('YYYY-MM-DD'),
      label: i === 0 ? `今天 ${d.format('M月D日')}` : `明天 ${d.format('M月D日')}`,
      events: (eventsList[i] || []).slice().sort((a, b) => a.start_ts - b.start_ts),
    }))
  } catch (e) {
    ElMessage.error(`加载日程失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

onMounted(load)
</script>

<style scoped>
.cdl {
  min-height: 80px;
}
.cdl-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.cdl-title {
  font-size: 13px;
  font-weight: 600;
  color: #303133;
  flex: 1;
}
.cdl-add {
  font-size: 12px;
  color: #67c23a;
  cursor: pointer;
  flex-shrink: 0;
}
.cdl-add:hover {
  text-decoration: underline;
}
.cdl-addform {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 10px;
  padding: 8px;
  background: #f8f9fb;
  border-radius: 8px;
}
.cdl-addrow {
  display: flex;
  align-items: center;
  gap: 10px;
}
.cdl-addrow .el-button {
  margin-left: auto;
}
.cdl-bell {
  flex-shrink: 0;
  font-size: 10px;
}
.cdl-open {
  font-size: 12px;
  color: #409eff;
  cursor: pointer;
}
.cdl-open:hover {
  text-decoration: underline;
}
.cdl-day {
  margin-bottom: 10px;
}
.cdl-date {
  font-size: 12px;
  color: #909399;
  margin-bottom: 6px;
}
.cdl-empty {
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
  color: #c0c4cc;
  font-size: 12px;
  text-align: center;
  padding: 12px 0;
}
.cdl-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 10px;
  margin-bottom: 5px;
  background: #fff;
  border: 1px solid #ebeef5;
  border-left: 3px solid #909399;
  border-radius: 6px;
}
.cdl-time {
  flex-shrink: 0;
  font-size: 11px;
  color: #909399;
  font-variant-numeric: tabular-nums;
}
.cdl-content {
  flex: 1;
  font-size: 13px;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.cdl-tag {
  flex-shrink: 0;
  font-size: 10px;
  color: #fff;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 4px;
}
</style>
