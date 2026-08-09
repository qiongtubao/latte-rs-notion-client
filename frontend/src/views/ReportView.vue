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
import { computed, ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { api } from '../api'

const configured = ref(true)
const period = ref('day')
const focus = ref('')
const compare = ref(false)
const loading = ref(false)
const saving = ref(false)
const report = ref(null)
const prevContext = ref('')
const errorMsg = ref('')
const savedTitle = ref('')

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
  savedTitle.value = ''
  try {
  const r = await api.aiReport(period.value, {
    focus: focus.value.trim(),
    compare: compare.value,
  });
  report.value = r;
  prevContext.value = r.prev_context || '';
  savedTitle.value = r.title || ''
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
.report-focus {
  width: 220px;
  flex: 1;
  max-width: 260px;
}
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
