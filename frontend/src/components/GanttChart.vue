<template>
  <el-empty v-if="projects.length === 0" description="还没有项目，先在看板里新建一个吧" :image-size="80" />

  <div v-else class="gantt">
    <div class="range-info">{{ rangeInfo }}</div>

    <div class="gantt-scroll">
      <div class="gantt-inner" :style="{ width: LEFT_W + totalW + 'px' }">
        <!-- 表头：月份分组 + 刻度（吸顶） -->
        <div class="g-header">
          <div class="g-corner">项目</div>
          <div class="g-header-right" :style="{ width: totalW + 'px' }">
            <div class="g-months">
              <div
                v-for="(m, i) in months"
                :key="i"
                class="g-month"
                :style="{ width: m.width + 'px' }"
              >{{ m.label }}</div>
            </div>
            <div class="g-ticks">
              <div
                v-for="(t, i) in ticks"
                :key="i"
                class="g-tick"
                :style="{ left: t.left + 'px', width: tickW + 'px' }"
              >{{ t.label }}</div>
            </div>
          </div>
        </div>

        <!-- 项目行 -->
        <div class="g-rows">
          <div class="today-line" :style="{ left: LEFT_W + todayLeft + 'px' }" />
          <div v-for="row in rows" :key="row.project.id" class="g-row">
            <div class="g-name" :title="row.project.name">{{ row.project.name }}</div>
            <div class="g-track" :style="{ width: totalW + 'px' }">
              <el-tooltip placement="top" :show-after="150">
                <template #content>
                  <div><b>{{ row.project.name }}</b> · {{ row.project.status }}</div>
                  <div v-if="row.scheduled">{{ fmtDate(row.start.unix()) }} ~ {{ fmtDate(row.end.unix()) }} · {{ row.days }} 天</div>
                  <div v-else>未设置日期（默认显示 今天~今天+7d），点击编辑</div>
                </template>
                <div
                  class="g-bar"
                  :class="{
                    doing: row.project.status === '进行中' && row.scheduled,
                    paused: row.project.status === '暂停',
                    unscheduled: !row.scheduled,
                  }"
                  :style="row.barStyle"
                  @click="emit('edit', row.project)"
                >
                  <span v-if="row.label" class="g-bar-text">{{ row.label }}</span>
                </div>
              </el-tooltip>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import dayjs from 'dayjs'
import { PROJECT_STATUS_COLORS, fmtDate } from '../utils'

const props = defineProps({
  projects: { type: Array, default: () => [] },
})
const emit = defineEmits(['edit'])

const LEFT_W = 180
const DAY_W = 36
const WEEK_W = 36

const todayStart = dayjs().startOf('day')

// 时间轴范围：min(start ?? 今天) ~ max(deadline ?? start+7d)，前后各 2 天 padding
const range = computed(() => {
  let min = null
  let max = null
  for (const p of props.projects) {
    const s = p.start_ts ? dayjs.unix(p.start_ts).startOf('day') : todayStart
    const e = p.deadline_ts ? dayjs.unix(p.deadline_ts).startOf('day') : s.add(7, 'day')
    if (!min || s.isBefore(min)) min = s
    if (!max || e.isAfter(max)) max = e
  }
  return { start: min.subtract(2, 'day'), end: max.add(2, 'day') }
})

const totalDays = computed(() => range.value.end.diff(range.value.start, 'day') + 1)
const weekMode = computed(() => totalDays.value > 62)
const pxPerDay = computed(() => (weekMode.value ? WEEK_W / 7 : DAY_W))
const tickW = computed(() => (weekMode.value ? WEEK_W : DAY_W))
const totalW = computed(() => Math.ceil(totalDays.value * pxPerDay.value))

const rangeInfo = computed(
  () =>
    `${range.value.start.format('YYYY-MM-DD')} ~ ${range.value.end.format('YYYY-MM-DD')} · 共 ${totalDays.value} 天${weekMode.value ? '（周刻度）' : ''}`
)

// 刻度：≤62 天按日，否则按周（标注周一）
const ticks = computed(() => {
  const list = []
  if (weekMode.value) {
    let d = range.value.start
    while (!d.isAfter(range.value.end)) {
      list.push({ label: d.format('M/d'), left: Math.round(d.diff(range.value.start, 'day') * pxPerDay.value) })
      d = d.add(7, 'day')
    }
  } else {
    for (let i = 0; i < totalDays.value; i++) {
      const d = range.value.start.add(i, 'day')
      list.push({ label: d.format('M/d'), left: i * DAY_W })
    }
  }
  return list
})

// 月份分组标签
const months = computed(() => {
  const list = []
  let cur = null
  for (let i = 0; i < totalDays.value; i++) {
    const d = range.value.start.add(i, 'day')
    const key = d.format('YYYY-M')
    if (!cur || cur.key !== key) {
      cur = { key, label: d.format('YYYY 年 M 月'), width: 0 }
      list.push(cur)
    }
    cur.width += pxPerDay.value
  }
  for (const m of list) m.width = Math.round(m.width)
  return list
})

const todayLeft = computed(() => Math.round(todayStart.diff(range.value.start, 'day') * pxPerDay.value))

// 项目行与色条布局
const rows = computed(() =>
  props.projects.map((p) => {
    const scheduled = !!(p.start_ts && p.deadline_ts)
    const start = scheduled ? dayjs.unix(p.start_ts).startOf('day') : todayStart
    const end = scheduled ? dayjs.unix(p.deadline_ts).startOf('day') : todayStart.add(7, 'day')
    const days = end.diff(start, 'day') + 1
    const left = Math.round(start.diff(range.value.start, 'day') * pxPerDay.value)
    const width = Math.max(Math.round(days * pxPerDay.value), 6)
    const color = PROJECT_STATUS_COLORS[p.status] || '#909399'
    return {
      project: p,
      scheduled,
      start,
      end,
      days,
      barStyle: scheduled
        ? { left: left + 'px', width: width + 'px', backgroundColor: color }
        : { left: left + 'px', width: width + 'px', backgroundColor: color + '22', border: `2px dashed ${color}`, color },
      label: width >= 60 ? `${p.status} · ${days}d` : width >= 28 ? `${days}d` : '',
    }
  })
)
</script>

<style scoped>
.range-info {
  color: #909399;
  font-size: 13px;
  margin-bottom: 10px;
}

.gantt-scroll {
  overflow: auto;
  max-height: 62vh;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  background: #fff;
}

.gantt-inner {
  position: relative;
}

/* 表头 */
.g-header {
  display: flex;
  position: sticky;
  top: 0;
  z-index: 6;
  background: #fff;
  border-bottom: 1px solid #e4e7ed;
}

.g-corner {
  flex: 0 0 180px;
  position: sticky;
  left: 0;
  z-index: 7;
  background: #fff;
  border-right: 1px solid #e4e7ed;
  padding: 8px 12px;
  font-weight: 600;
  display: flex;
  align-items: flex-end;
}

.g-months {
  display: flex;
  border-bottom: 1px solid #f0f0f0;
}

.g-month {
  font-size: 12px;
  color: #606266;
  padding: 4px 6px;
  white-space: nowrap;
  overflow: hidden;
  border-right: 1px solid #f0f0f0;
}

.g-ticks {
  position: relative;
  height: 26px;
}

.g-tick {
  position: absolute;
  top: 0;
  font-size: 11px;
  color: #909399;
  text-align: center;
  line-height: 26px;
  border-right: 1px solid #f5f5f5;
  overflow: hidden;
  white-space: nowrap;
}

/* 行 */
.g-rows {
  position: relative;
}

.g-row {
  display: flex;
  height: 36px;
}

.g-row:nth-child(even) {
  background: #fafafa;
}

.g-row:hover {
  background: #ecf5ff;
}

.g-name {
  flex: 0 0 180px;
  position: sticky;
  left: 0;
  z-index: 5;
  background: inherit;
  border-right: 1px solid #e4e7ed;
  padding: 0 12px;
  font-size: 13px;
  line-height: 1.3;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  align-items: center;
  word-break: break-all;
}

/* sticky 单元格需要实底，斑马纹与 hover 同步到名称列 */
.g-row:nth-child(odd) .g-name {
  background: #fff;
}

.g-row:nth-child(even) .g-name {
  background: #fafafa;
}

.g-row:hover .g-name {
  background: #ecf5ff;
}

.g-track {
  position: relative;
}

.g-bar {
  position: absolute;
  top: 6px;
  height: 24px;
  border-radius: 6px;
  cursor: pointer;
  color: #fff;
  font-size: 12px;
  line-height: 24px;
  padding: 0 8px;
  overflow: hidden;
  white-space: nowrap;
  box-sizing: border-box;
}

.g-bar.doing {
  background-image: repeating-linear-gradient(
    45deg,
    rgba(255, 255, 255, 0.25) 0 8px,
    transparent 8px 16px
  );
  animation: stripe-slide 1.2s linear infinite;
}

@keyframes stripe-slide {
  from {
    background-position: 0 0;
  }
  to {
    background-position: 22px 0;
  }
}

.g-bar.paused {
  opacity: 0.55;
}

.g-bar.unscheduled {
  line-height: 20px;
}

.g-bar-text {
  pointer-events: none;
}

.today-line {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  background: #f56c6c;
  z-index: 4;
  pointer-events: none;
}
</style>
