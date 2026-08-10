<template>
  <div class="report-view">
    <!-- 顶部控制条 -->
    <div class="report-toolbar">
      <el-radio-group v-model="period" size="small">
        <el-radio-button value="day">日报</el-radio-button>
        <el-radio-button value="week">周报</el-radio-button>
        <el-radio-button value="month">月报</el-radio-button>
      </el-radio-group>
      <el-input
        v-model="focus"
        :placeholder="period === 'day' ? '重点想关注什么？（可选）' : period === 'week' ? '本周重点？（可选）' : '本月重点？（可选）'"
        size="small"
        clearable
        class="report-focus"
        @keyup.enter="generate"
      />
      <el-checkbox v-model="compare" size="small" title="把上一同期数据也喂给 AI，让它做趋势对比">对比上一期</el-checkbox>
      <el-button type="primary" size="small" :loading="loading" :disabled="!configured" @click="generate">
      </el-button>
      <el-button
        v-if="report"
        size="small"
        type="success"
        :loading="saving"
        :disabled="!savedTitle"
        @click="saveAsNote"
      >存为知识库文档</el-button>
    </div>

    <div v-if="!configured" class="report-hint">
      请先在设置里配置 AI（latte-model-proxy）。
    </div>

    <!-- 原始数据兜底提示（AI 不可用也可看数据汇总） -->
    <div v-if="report" class="report-body">
      <div class="report-title">{{ report.title }}</div>
      <div class="markdown-body" v-html="rendered" />
      <!-- 对比图表：本周期 vs 上一周期 -->
      <div v-if="stats" class="compare-chart">
        <div class="compare-title">📊 本期 vs 上一期</div>
        <div class="cmp-row" v-for="row in chartRows" :key="row.label">
          <span class="cmp-label">{{ row.label }}</span>
          <div class="cmp-bars">
            <div class="cmp-bargroup">
              <div class="cmp-bar" :class="row.down ? 'down' : ''" :style="{ width: row.curPct + '%' }" :title="`本期 ${row.curText}`" />
              <span class="cmp-val">{{ row.curText }}</span>
            </div>
            <div class="cmp-bargroup">
              <div class="cmp-bar prev" :style="{ width: row.prevPct + '%' }" :title="`上期 ${row.prevText}`" />
              <span class="cmp-val">{{ row.prevText }}</span>
            </div>
          </div>
        </div>
        <div class="cmp-legend"><span class="leg cur">本期</span><span class="leg prev">上一期</span></div>
      </div>
      <el-collapse v-if="prevContext || report.context" class="report-raw-collapse">
        <el-collapse-item v-if="prevContext" title="上一同期数据汇总（对比模式）">
          <pre class="report-raw">{{ prevContext }}</pre>
        </el-collapse-item>
        <el-collapse-item title="查看 AI 依据的原始数据汇总">
          <pre class="report-raw">{{ report.context }}</pre>
        </el-collapse-item>
      </el-collapse>
    </div>
    <div v-else-if="loading" v-loading="true" class="report-loading" />
    <el-empty v-else description="选择周期后点击「生成」来产出日报 / 周报 / 月报" :image-size="80" />

    <div v-if="errorMsg" class="report-error">{{ errorMsg }}</div>
  </div>
</template>

<script setup>

function fmtSecs(s) {
  if (s == null) return '-'
  const h = Math.floor(s / 3600), m = Math.round((s % 3600) / 60)
  return h > 0 ? `${h}h${m}m` : `${m}m`
}
function fmtYuan2(cents) {
  return cents == null ? '-' : `¥${(cents / 100).toFixed(1)}`
}
// 图表行：两期对比，横向条按较大值归一化
const chartRows = computed(() => {
  const st = stats.value
  if (!st || !st.current || !st.prev) return []
  const c = st.current, p = st.prev
  const rows = [
    { label: '时间投入', cur: c.time_secs || 0, prev: p.time_secs || 0, curText: fmtSecs(c.time_secs), prevText: fmtSecs(p.time_secs) },
    { label: '消费', cur: c.expense_cents || 0, prev: p.expense_cents || 0, curText: fmtYuan2(c.expense_cents), prevText: fmtYuan2(p.expense_cents), down: true },
  ]
  const curDone = c.tasks_done || 0, prevDone = p.tasks_done || 0
  rows.push({ label: '完成任务', cur: curDone, prev: prevDone, curText: String(curDone), prevText: String(prevDone) })
  return rows.map(r => {
    const max = Math.max(r.cur, r.prev, 1)
    return { ...r, curPct: Math.round(r.cur / max * 100), prevPct: Math.round(r.prev / max * 100) }
  })
})
const compare = ref(false)
const loading = ref(false)
const saving = ref(false)
const report = ref(null)
const prevContext = ref('')
const savedTitle = ref('')
const stats = ref(null)

const PERIOD_LABEL = { day: '日报', week: '周报', month: '月报' }

const rendered = computed(() => {
  if (!report.value) return ''
  return DOMPurify.sanitize(marked.parse(report.value.body_md || ''))
})

watch(period, () => { report.value = null; errorMsg.value = '' })

async function generate() {
  loading.value = true
  errorMsg.value = ''
  report.value = null
  prevContext.value = ''
  savedTitle.value = ''
  stats.value = null
  try {
    const r = await api.aiReport(period.value, {
      focus: focus.value.trim(),
      compare: compare.value,
    });
    report.value = r;
    prevContext.value = r.prev_context || '';
    savedTitle.value = r.title || ''
    // 对比模式额外拉结构化指标给图表
    if (compare.value) {
      stats.value = await api.getReportStats(period.value)
    }
  } catch (e) {
    errorMsg.value = e.status === 502
      ? `${e.message}（请检查 latte-model-proxy 是否启动）`
      : e.message
  } finally {
    loading.value = false
  }
}

async function saveAsNote() {
  if (!report.value) return
  saving.value = true
  try {
    await api.createNote({ title: report.value.title || `${PERIOD_LABEL[period.value]}说明`, content_md: report.value.body_md, kind: 'doc' })
    ElMessage.success('已存入知识库')
    savedTitle.value = report.value.title || ''
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}
</script>

<style scoped>
.report-view {
  padding: 4px;
  height: 100%;
  display: flex;
  flex-direction: column;
}
.report-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
  flex-wrap: wrap;
}

/* 对比图表 */
.compare-chart {
  margin: 12px 0 4px;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 10px 14px;
}
.compare-title { font-weight: 600; margin-bottom: 8px; font-size: 14px; }
.cmp-row {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.cmp-label { flex: 0 0 72px; font-size: 13px; color: var(--el-text-color-regular); }
.cmp-bars { flex: 1; display: flex; flex-direction: column; gap: 4px; }
.cmp-bargroup { display: flex; align-items: center; gap: 6px; }
.cmp-bar {
  height: 12px;
  min-width: 2px;
  border-radius: 3px;
  background: #409eff;
  transition: width 0.3s ease;
}
.cmp-bar.down { background: #f56c6c; }
.cmp-bar.prev { background: #c0c4cc; }
.cmp-val { font-size: 12px; color: var(--el-text-color-secondary); font-variant-numeric: tabular-nums; }
.cmp-legend { display: flex; gap: 14px; font-size: 12px; margin-top: 6px; color: var(--el-text-color-regular); }
.cmp-legend .leg { display: inline-flex; align-items: center; gap: 4px; }
.cmp-legend .leg::before { content: ''; width: 10px; height: 10px; border-radius: 2px; display: inline-block; }
.cmp-legend .leg.cur::before { background: #409eff; }
.cmp-legend .leg.prev::before { background: #c0c4cc; }
.report-hint {
  color: var(--el-color-warning);
  font-size: 13px;
  margin: 8px 0;
}
.report-body {
  flex: 1;
  overflow: auto;
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 12px 16px;
  background: var(--el-fill-color-blank);
}
.report-title {
  font-size: 18px;
  font-weight: 600;
  margin-bottom: 10px;
}
.report-loading {
  flex: 1;
  min-height: 200px;
}
.report-error {
  color: var(--el-color-danger);
  font-size: 13px;
  margin-top: 8px;
  white-space: pre-wrap;
}
.report-raw-collapse {
  margin-top: 16px;
}
.report-raw {
  white-space: pre-wrap;
  font-size: 12px;
  color: var(--el-text-color-regular);
  margin: 0;
  max-height: 260px;
  overflow: auto;
}
</style>
