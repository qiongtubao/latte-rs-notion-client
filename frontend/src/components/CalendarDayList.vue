<template>
  <div class="cdl" v-loading="loading">
    <div class="cdl-header">
      <span class="cdl-title">日程</span>
      <span class="cdl-open" @click="openFull">打开完整日历 →</span>
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
        <span class="cdl-content" :title="e.content">{{ e.content || '(无内容)' }}</span>
        <span v-if="e.tag" class="cdl-tag" :style="{ background: tagColor(e.tag) }">{{ e.tag }}</span>
      </div>
    </div>
  </div>
</template>

<script setup>
import { inject, onMounted, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'

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
  margin-bottom: 8px;
}
.cdl-title {
  font-size: 13px;
  font-weight: 600;
  color: #303133;
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
