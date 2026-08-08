<template>
  <el-dialog
    :model-value="open"
    :title="title"
    width="560px"
    :close-on-click-modal="false"
    @open="onOpen"
    @update:model-value="(v) => { if (!v) emit('close') }"
  >
    <el-input
      v-model="text"
      type="textarea"
      :rows="3"
      :placeholder="placeholder"
      :disabled="busy"
      @keydown.ctrl.enter.prevent="run"
    />
    <div class="ai-hint">按 Ctrl/⌘+Enter 直接生成；模型不可用时右侧会给出原因。</div>

    <div v-if="summary" class="ai-summary">
      <div class="ai-summary-label">AI 建议</div>
      <div class="ai-summary-body">{{ summary }}</div>
    </div>

    <div v-if="items.length" class="ai-items">
      <div class="ai-items-label">可采纳条目（{{ items.length }}）</div>
      <div v-for="(it, idx) in items" :key="idx" class="ai-card">
        <div class="ai-card-head">
          <span class="ai-type">{{ typeLabel(it.type) }}</span>
          <span class="ai-card-title">{{ cardTitle(it) }}</span>
        </div>
        <div class="ai-card-body">
          <div v-for="f in cardFields(it)" :key="f.k" class="ai-field">
            <span class="ai-k">{{ f.k }}</span>
            <span>{{ f.v }}</span>
          </div>
        </div>
        <div class="ai-card-actions">
          <el-button size="small" type="primary" :loading="adoptingIdx === idx" @click="adopt(it, idx)">
            采纳
          </el-button>
          <el-button size="small" text :disabled="adoptingIdx === idx" @click="discard(idx)">
            丢弃
          </el-button>
        </div>
      </div>
    </div>

    <div v-if="errorMsg" class="ai-error">{{ errorMsg }}</div>

    <template #footer>
      <el-button @click="emit('close')">关闭</el-button>
      <el-button type="primary" :loading="busy" :disabled="!text.trim()" @click="run">生成</el-button>
    </template>
  </el-dialog>
</template>

<script setup>
import { ref, watch } from 'vue'
import { ElMessage } from 'element-plus'
import { api } from '../api'

const props = defineProps({
  open: { type: Boolean, default: false },
  panel: { type: String, required: true },
  title: { type: String, default: '🤖 AI 助手' },
  placeholder: { type: String, default: '说说你的需求…' },
  context: { type: String, default: '' },
})
const emit = defineEmits(['close', 'adopted'])

const text = ref('')
const busy = ref(false)
const adoptingIdx = ref(-1)
const summary = ref('')
const items = ref([])
const errorMsg = ref('')

watch(() => props.open, (v) => {
  if (v) {
    text.value = ''
    summary.value = ''
    items.value = []
    errorMsg.value = ''
    adoptingIdx.value = -1
  }
})

function onOpen() {
  // dialog 打开时把焦点交给 textarea
  // 留给用户，不强求自动 focus
}

const TYPE_LABELS = {
  task: '☑ 任务',
  event: '📅 日程',
  expense: '💰 消费',
  idea: '💡 想法',
  note: '📚 文档',
  project: '📊 项目',
  daily_item: '✅ 打卡项',
}
function typeLabel(t) { return TYPE_LABELS[t] || t }

function dt(s) {
  // 把 "YYYY-MM-DD HH:mm" 渲染成 "MM-DD HH:mm"
  if (!s) return ''
  const m = s.match(/^\d{4}-(\d{2})-(\d{2})(?:[ T](\d{2}):(\d{2}))?/)
  if (!m) return s
  return `${m[1]}-${m[2]}${m[3] ? ' ' + m[3] + ':' + m[4] : ''}`
}

function cardTitle(it) {
  switch (it.type) {
    case 'task': return it.title
    case 'event': return it.content
    case 'expense': return it.item
    case 'idea': return it.content
    case 'note': return it.title
    case 'project': return it.name
    case 'daily_item': return it.name
    default: return ''
  }
}

function cardFields(it) {
  switch (it.type) {
    case 'task':
      return [
        { k: '日期', v: it.date || '今天' },
        { k: '优先级', v: it.priority || '中' },
      ]
    case 'project': return [{ k: '目标', v: it.description || '（无）' }]
    case 'event':
      return [
        { k: '标签', v: it.tag },
        { k: '开始', v: dt(it.start) },
        { k: '结束', v: it.end ? dt(it.end) : '（默认 1 小时）' },
        { k: '提醒', v: it.remind ? '🔔 是' : '否' },
      ]
    case 'expense':
      return [
        { k: '金额', v: `¥${it.amount}` },
        { k: '分类', v: it.category },
        { k: '时间', v: it.time ? dt(it.time) : '现在' },
      ]
    case 'idea':
      return [{ k: '标签', v: it.tag }]
    case 'note':
      return [
        { k: '类型', v: it.kind || 'doc' },
        { k: '预览', v: (it.content_md || '').slice(0, 80) + ((it.content_md || '').length > 80 ? '…' : '') },
      ]
    case 'project':
      return [{ k: '目标', v: it.description || '（无）' }]
    case 'daily_item':
      return [
        { k: '类型', v: it.kind },
        { k: '单位', v: it.unit || '（无）' },
      ]
    default:
      return []
  }
}

async function run() {
  const t = text.value.trim()
  if (!t) return
  busy.value = true
  errorMsg.value = ''
  summary.value = ''
  items.value = []
  try {
    const r = await api.aiAssist(props.panel, t, props.context)
    summary.value = r.summary || ''
    items.value = Array.isArray(r.items) ? r.items : []
    if (!summary.value && !items.value.length) {
      errorMsg.value = 'AI 没有给出建议，试试描述得更具体一些。'
    }
  } catch (e) {
    errorMsg.value = e.status === 502
      ? `${e.message}（请检查 latte-model-proxy 是否启动）`
      : e.message
  } finally {
    busy.value = false
  }
}

// 调各 create 接口；返回后由调用方决定是否要全局刷新
async function adoptOne(it) {
  switch (it.type) {
    case 'task':
      return api.createTask({ title: it.title, date: it.date || undefined, priority: it.priority || '中' })
    case 'event': {
      // 解析 "YYYY-MM-DD HH:mm" → unix 秒
      const parseTs = (s) => {
        if (!s) return undefined
        const m = s.match(/^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/)
        if (!m) return undefined
        return Math.floor(new Date(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:00`).getTime() / 1000)
      }
      const start_ts = parseTs(it.start)
      if (!start_ts) throw new Error('事件缺少合法的开始时间')
      const end_ts = parseTs(it.end) ?? (start_ts + 3600)
      return api.createEvent({ content: it.content, tag: it.tag || '生活', start_ts, end_ts, remind: !!it.remind })
    }
    case 'expense': {
      const parseTs = (s) => {
        if (!s) return undefined
        const m = s.match(/^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/)
        if (!m) return undefined
        return Math.floor(new Date(`${m[1]}-${m[2]}-${m[3]}T${m[4]}:${m[5]}:00`).getTime() / 1000)
      }
      return api.createExpense({
        item: it.item, amount: it.amount, category: it.category || '其他',
        ts: parseTs(it.time) ?? Math.floor(Date.now() / 1000),
      })
    }
    case 'idea':
      return api.createIdea({ content: it.content, tag: it.tag || '灵感' })
    case 'note':
      return api.createNote({ title: it.title, content_md: it.content_md || '', kind: it.kind || 'doc' })
    case 'project':
      return api.createProject({ name: it.name, note: it.description || '' })
    case 'daily_item':
      return api.createDailyItem({ name: it.name, kind: it.kind || '打卡', unit: it.unit || '' })
    default:
      throw new Error(`未知类型：${it.type}`)
  }
}

async function adopt(it, idx) {
  adoptingIdx.value = idx
  try {
    await adoptOne(it)
    ElMessage.success(`${typeLabel(it.type)} 已创建`)
    items.value.splice(idx, 1)
    emit('adopted', it)
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adoptingIdx.value = -1
  }
}

function discard(idx) {
  items.value.splice(idx, 1)
}
</script>

<style scoped>
.ai-hint {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin: 6px 0 8px;
}
.ai-summary {
  background: var(--el-color-primary-light-9);
  border: 1px solid var(--el-color-primary-light-7);
  border-radius: 6px;
  padding: 8px 12px;
  margin-bottom: 10px;
  white-space: pre-wrap;
  font-size: 13px;
}
.ai-summary-label {
  font-weight: 600;
  margin-bottom: 4px;
  color: var(--el-color-primary);
}
.ai-items-label {
  font-size: 12px;
  color: var(--el-text-color-secondary);
  margin: 6px 0 4px;
}
.ai-card {
  border: 1px solid var(--el-border-color-lighter);
  border-radius: 6px;
  padding: 8px 10px;
  margin-bottom: 8px;
  background: var(--el-fill-color-blank);
}
.ai-card-head {
  display: flex;
  align-items: center;
  gap: 8px;
}
.ai-type {
  display: inline-block;
  font-size: 12px;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--el-color-info-light-9);
  color: var(--el-color-info);
}
.ai-card-title {
  font-weight: 500;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ai-card-body {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 4px 12px;
  margin: 6px 0;
  font-size: 12px;
  color: var(--el-text-color-regular);
}
.ai-field { display: flex; gap: 4px; }
.ai-k { color: var(--el-text-color-secondary); }
.ai-card-actions {
  display: flex;
  gap: 6px;
  justify-content: flex-end;
}
.ai-error {
  color: var(--el-color-danger);
  font-size: 13px;
  margin-top: 4px;
}
</style>
