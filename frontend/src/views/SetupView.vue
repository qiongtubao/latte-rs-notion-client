<template>
  <div class="setup-page">
    <el-card class="setup-card">
      <div class="setup-head">
        <h2>{{ pageTitle }}</h2>
        <el-button v-if="closable" size="small" @click="emit('done')">← 返回</el-button>
      </div>
      <p v-if="closable" class="sub">
        已自动加载你 Notion 根页面下的全部数据库及数据条数，可直接进行下方操作。
      </p>
      <p v-else class="sub">首次使用需要连接你的 Notion，请按以下步骤操作：</p>

      <ol v-if="!closable" class="steps">
        <li>
          打开
          <el-link type="primary" href="https://www.notion.so/profile/integrations" target="_blank">
            notion.so/profile/integrations
          </el-link>
          ，创建一个新的 Integration，复制它的 <b>Internal Integration Secret</b>（以 <code>ntn_</code> 或 <code>secret_</code> 开头）。
        </li>
        <li>
          在 Notion 中新建一个页面（作为 Latte 的数据库根页面），点击右上角 <b>···</b> →
          <b>连接 / Connect to</b> → 选择刚才创建的 Integration。
        </li>
        <li>在下方粘贴 Token 和页面 URL，点击「测试配置」。</li>
      </ol>

      <el-form v-if="!closable" :model="form" label-width="90px" @submit.prevent>
        <el-form-item label="Token">
          <el-input v-model="form.token" placeholder="ntn_xxxxxxxxxxxx" show-password />
        </el-form-item>
        <el-form-item label="页面 URL">
          <el-input v-model="form.page_url" placeholder="https://www.notion.so/xxxx" />
        </el-form-item>
        <el-form-item>
          <el-button type="default" :loading="testing" @click="testConfig">测试配置</el-button>
          <el-button type="primary" :loading="submitting" @click="submit">完成配置</el-button>
        </el-form-item>
        <!-- 测试结果展示 -->
        <el-alert v-if="testResult" :title="testResult.title" :type="testResult.type" :closable="false" show-icon class="err">
          <template #default>
            <div v-if="testResult.detail" style="margin-top:4px; font-size:13px; line-height:1.6;">{{ testResult.detail }}</div>
            <div v-if="testResult.databases && testResult.databases.length" style="margin-top:4px; font-size:13px;">
              已找到数据库：
              <el-tag v-for="db in testResult.databases" :key="db.id" size="small" style="margin:2px 4px 2px 0">
                {{ db.title }}
              </el-tag>
            </div>
            <div v-if="testResult.missing && testResult.missing.length" style="margin-top:4px; font-size:13px;">
              未找到数据库：
              <el-tag v-for="m in testResult.missing" :key="m" type="warning" size="small" style="margin:2px 4px 2px 0">
                {{ m }}
              </el-tag>
            </div>
          </template>
        </el-alert>
        <el-alert v-if="error" :title="error" type="error" :closable="false" class="err" />
      </el-form>

      <!-- 管理模式：免填 token，用已保存配置直接加载 -->
      <template v-else>
        <div v-if="loadingCandidates" class="sub" style="margin: 8px 0;">
          <el-icon class="is-loading"><Loading /></el-icon> 正在加载数据库…
        </div>
        <el-alert v-if="error" :title="error" type="error" :closable="false" class="err" />
        <el-button size="small" :loading="loadingCandidates" @click="loadCandidates">刷新</el-button>
      </template>

      <!-- 数据库选择（重新绑定） -->
      <template v-if="candidates.length > 0 && showRebind">
        <el-divider />
        <h3>重新绑定数据库</h3>
        <p class="sub" v-if="closable">为每个类型选择要绑定的库，点击「保存绑定」生效；留空则自动选择 / 创建新库。</p>
        <p class="sub" v-else>如果同一个类型发现多个库，请手动选择要使用的库；留空则自动选择。</p>
        <el-form label-width="110px">
          <el-form-item v-for="k in kindList" :key="k.key" :label="k.label">
            <el-select v-model="selectedIds[k.key]" clearable placeholder="自动选择 / 创建新库" style="width: 100%;">
              <el-option label="自动选择 / 创建新库" value="" />
              <el-option
                v-for="c in candidatesByKind(k.key)"
                :key="c.id"
                :label="`${c.title} (${c.row_count}条)${c.schema_ok ? '' : ' [结构不匹配]'}`"
                :value="c.id"
              />
            </el-select>
          </el-form-item>
          <el-form-item v-if="closable">
            <el-button type="primary" :loading="submitting" @click="submit">保存绑定</el-button>
          </el-form-item>
        </el-form>
      </template>

      <!-- 合并数据库 -->
      <template v-if="candidates.length > 0 && showMerge">
        <el-divider />
        <h3>合并数据库</h3>
        <p class="sub">勾选同类型的多个库，把数据复制到其中一个目标库。</p>
        <el-table :data="candidates" style="width: 100%; margin-top: 12px;" size="small" @selection-change="onSelectionChange">
          <el-table-column type="selection" width="40" />
          <el-table-column prop="title" label="标题" />
          <el-table-column label="类型" width="100">
            <template #default="{ row }">{{ kindLabel(row.kind) }}</template>
          </el-table-column>
          <el-table-column label="行数" width="80">
            <template #default="{ row }">{{ row.row_count }}</template>
          </el-table-column>
          <el-table-column label="结构" width="150">
            <template #default="{ row }">
              <el-tag v-if="row.schema_ok" type="success" size="small">匹配</el-tag>
              <template v-else>
                <el-tag type="warning" size="small">不匹配</el-tag>
                <el-button type="warning" size="small" link :loading="fixingId === row.id" @click="fixSchema(row)">修复</el-button>
              </template>
            </template>
          </el-table-column>
        </el-table>
        <div style="margin-top: 12px;">
          <el-button type="primary" size="small" :disabled="!canMerge" @click="openMergeDialog">合并选中库</el-button>
        </div>
      </template>

      <!-- 删除数据库 -->
      <template v-if="candidates.length > 0 && showDelete">
        <el-divider />
        <h3>删除数据库</h3>
        <p class="sub">删除后会被移到 Notion 回收站，用于清理空库或重复库；可勾选多个一键删除。</p>
        <el-table :data="candidates" style="width: 100%; margin-top: 12px;" size="small" @selection-change="onDeleteSelectionChange">
          <el-table-column type="selection" width="40" />
          <el-table-column prop="title" label="标题" />
          <el-table-column label="类型" width="100">
            <template #default="{ row }">{{ kindLabel(row.kind) }}</template>
          </el-table-column>
          <el-table-column label="行数" width="80">
            <template #default="{ row }">{{ row.row_count }}</template>
          </el-table-column>
          <el-table-column label="结构" width="150">
            <template #default="{ row }">
              <el-tag v-if="row.schema_ok" type="success" size="small">匹配</el-tag>
              <template v-else>
                <el-tag type="warning" size="small">不匹配</el-tag>
                <el-button type="warning" size="small" link :loading="fixingId === row.id" @click="fixSchema(row)">修复</el-button>
              </template>
            </template>
          </el-table-column>
          <el-table-column label="操作" width="120">
            <template #default="{ row }">
              <el-button type="danger" size="small" @click="deleteDb(row)">删除</el-button>
            </template>
          </el-table-column>
        </el-table>
        <div style="margin-top: 12px;">
          <el-button type="danger" size="small" :disabled="!deleteSelection.length" :loading="batchDeleting" @click="deleteSelected">
            一键删除选中库（{{ deleteSelection.length }}）
          </el-button>
        </div>
      </template>
    </el-card>

    <!-- 合并对话框 -->
    <el-dialog v-model="mergeDialogVisible" title="合并数据库" width="420px">
      <p>将把以下库的数据复制到目标库：</p>
      <ul>
        <li v-for="c in selectedCandidates" :key="c.id">{{ c.title }} ({{ c.row_count }}条)</li>
      </ul>
      <el-form label-width="100px" style="margin-top: 12px;">
        <el-form-item label="目标库">
          <el-select v-model="mergeTargetId" placeholder="选择目标库" style="width: 100%;">
            <el-option v-for="c in selectedCandidates" :key="c.id" :label="c.title" :value="c.id" />
          </el-select>
        </el-form-item>
        <el-form-item>
          <el-checkbox v-model="mergeDeleteSources">合并后删除源库</el-checkbox>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="mergeDialogVisible = false">取消</el-button>
        <el-button type="primary" :loading="merging" @click="doMerge">确认合并</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { reactive, ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Loading } from '@element-plus/icons-vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { invoke } from '@tauri-apps/api/core'
import { api } from '../api'

// 已配置后从「设置」打开时 closable 为 true：显示返回按钮、隐藏新手引导；
// section 为空时显示全部区块（首次配置流程），否则只显示对应功能区
const props = defineProps({
  closable: { type: Boolean, default: false },
  section: { type: String, default: '' }, // '' / merge / delete / rebind
})
const showRebind = computed(() => !props.section || props.section === 'rebind')
const showMerge = computed(() => !props.section || props.section === 'merge')
const showDelete = computed(() => !props.section || props.section === 'delete')
const pageTitle = computed(() => {
  if (!props.closable) return '欢迎使用 Latte'
  return { merge: '合并数据库', delete: '删除数据库', rebind: '重新绑定数据库' }[props.section] || '数据库管理'
})
const emit = defineEmits(['done'])

async function notifySetupDone() {
  try {
    const win = getCurrentWindow()
    if (win.label === 'setup') {
      await invoke('setup_done')
    }
  } catch {
    // Web 版或非 Tauri 环境忽略
  }
}

const form = reactive({ token: '', page_url: '' })
const error = ref('')
const submitting = ref(false)
const testing = ref(false)
const testResult = ref(null)
const candidates = ref([])
const merging = ref(false)
const mergeDialogVisible = ref(false)
const mergeTargetId = ref('')
const mergeDeleteSources = ref(false)

const kindList = [
  { key: 'events', label: '时间碎片' },
  { key: 'expenses', label: '金钱记录' },
  { key: 'projects', label: '项目管理' },
  { key: 'notes', label: '📚 知识库' },
]

const selectedIds = reactive({
  events: '',
  expenses: '',
  projects: '',
  notes: '',
})

const mergeSelection = ref([])
const deleteSelection = ref([])
const batchDeleting = ref(false)
const fixingId = ref('')

// 修复远端库结构：补齐缺失字段、title 列重命名；类型冲突的字段提示手动处理
async function fixSchema(row) {
  try {
    await ElMessageBox.confirm(
      `将按 Latte 标准结构修复「${row.title}」：补齐缺失的字段（含 select 选项）、必要时重命名标题列。已有数据和多余字段不会改动。`,
      '修复数据库结构',
      { confirmButtonText: '修复', cancelButtonText: '取消', type: 'info' }
    )
  } catch {
    return
  }
  fixingId.value = row.id
  try {
    const r = await api.fixDatabase({ token: form.token.trim(), database_id: row.id })
    const parts = []
    if (r.added?.length) parts.push(`已补齐字段：${r.added.join('、')}`)
    if (r.renamed_title) parts.push(`标题列「${r.renamed_title[0]}」已重命名为「${r.renamed_title[1]}」`)
    if (!parts.length && !r.conflicts?.length) parts.push('结构已匹配，无需修改')
    if (r.conflicts?.length) {
      parts.push(`以下字段类型不一致，请在 Notion 中手动处理：${r.conflicts.join('、')}`)
      ElMessage.warning(parts.join('；'))
    } else {
      ElMessage.success(parts.join('；'))
    }
    await refreshCandidates()
  } catch (e) {
    ElMessage.error(e.message || '修复失败')
  } finally {
    fixingId.value = ''
  }
}

function onDeleteSelectionChange(rows) {
  deleteSelection.value = rows
}

// 一键删除选中库：逐个调用删除接口（后端已限速），全部完成后统一刷新
async function deleteSelected() {
  const list = deleteSelection.value
  if (!list.length) return
  const totalRows = list.reduce((sum, c) => sum + (c.row_count || 0), 0)
  try {
    await ElMessageBox.confirm(
      `确定删除选中的 ${list.length} 个数据库吗？共 ${totalRows} 条数据，删除后都会移到 Notion 回收站。\n\n` +
        list.map((c) => `· ${c.title}（${c.row_count}条）`).join('\n'),
      '一键删除确认',
      { confirmButtonText: '全部删除', cancelButtonText: '取消', type: 'warning' }
    )
  } catch {
    return
  }
  batchDeleting.value = true
  let ok = 0
  const failed = []
  for (const c of list) {
    try {
      await api.deleteDatabase({ token: form.token.trim(), database_id: c.id })
      ok++
    } catch (e) {
      failed.push(`${c.title}: ${e.message}`)
    }
  }
  batchDeleting.value = false
  if (failed.length) {
    ElMessage.error(`已删除 ${ok} 个，失败 ${failed.length} 个：${failed.join('；')}`)
  } else {
    ElMessage.success(`已删除 ${ok} 个数据库`)
  }
  await refreshCandidates()
}
const loadingCandidates = ref(false)

// 管理模式：用后端已保存的 token/页面直接加载候选库（含行数），无需重填配置
async function loadCandidates() {
  loadingCandidates.value = true
  error.value = ''
  try {
    const res = await api.setupCandidates()
    if (!res.token_valid) throw new Error('已保存的 Token 无效，请重新完成初始配置')
    if (res.error) throw new Error(res.error)
    candidates.value = res.candidates || []
    mergeSelection.value = []
    deleteSelection.value = []
  } catch (e) {
    error.value = e.message || '加载数据库失败'
    candidates.value = []
  } finally {
    loadingCandidates.value = false
  }
}

onMounted(() => {
  if (props.closable) loadCandidates()
})

// 操作后刷新列表：管理模式走免 token 接口，首次配置走 verify
async function refreshCandidates() {
  if (props.closable) await loadCandidates()
  else await testConfig()
}

const selectedCandidates = computed(() =>
  candidates.value.filter((c) => mergeSelection.value.includes(c.id))
)

const canMerge = computed(() => {
  const list = selectedCandidates.value
  if (list.length < 2) return false
  const kind = list[0].kind
  return list.every((c) => c.kind === kind)
})

function kindLabel(kind) {
  const map = {
    events: '时间碎片',
    expenses: '金钱记录',
    projects: '项目管理',
    notes: '📚 知识库',
    ideas: '好想法',
    tasks: '今日任务',
    daily: '✅ 每日打卡',
  }
  return map[kind] || kind
}

function candidatesByKind(kind) {
  return candidates.value.filter((c) => c.kind === kind)
}

function onSelectionChange(rows) {
  mergeSelection.value = rows.map((r) => r.id)
}

async function testConfig() {
  if (!form.token.trim() || !form.page_url.trim()) {
    error.value = '请填写 Token 和页面 URL'
    return
  }
  error.value = ''
  testResult.value = null
  testing.value = true
  candidates.value = []
  try {
    const res = await api.verifySetup(form.token.trim(), form.page_url.trim())
    candidates.value = res.candidates || []
    // 清空已选
    selectedIds.events = ''
    selectedIds.expenses = ''
    selectedIds.projects = ''
    selectedIds.notes = ''
    mergeSelection.value = []

    if (!res.token_valid) {
      testResult.value = {
        type: 'error',
        title: 'Token 无效或无权访问',
        detail: '请检查 Token 是否正确，以及 Integration 是否已连接到目标页面。',
        databases: [],
        missing: ['时间碎片', '金钱记录', '项目管理'],
      }
    } else if (res.error) {
      testResult.value = {
        type: 'error',
        title: '无法访问该页面',
        detail: res.error,
        databases: [],
        missing: ['时间碎片', '金钱记录', '项目管理'],
      }
    } else {
      const climbed = res.is_database
        ? '检测到你填的 URL 指向的是一个数据库，已自动向上定位到它的父页面。'
        : ''
      if (res.missing && res.missing.length) {
        testResult.value = {
          type: 'warning',
          title: 'Token 有效，但部分数据库未找到',
          detail: (climbed ? climbed + ' ' : '') +
            '以下数据库未找到，点击「完成配置」将自动创建缺失的数据库；也可在上方手动选择已有库。',
          databases: res.databases,
          missing: res.missing,
        }
      } else {
        testResult.value = {
          type: 'success',
          title: '配置正确！所有数据库已就绪',
          detail: (climbed ? climbed + ' ' : '') +
            'Token 有效，已找到全部数据库。',
          databases: res.databases,
          missing: [],
        }
      }
    }
  } catch (e) {
    testResult.value = {
      type: 'error',
      title: '验证失败',
      detail: e.message,
      databases: [],
      missing: ['时间碎片', '金钱记录', '项目管理'],
    }
  } finally {
    testing.value = false
  }
}

async function deleteDb(row) {
  try {
    await ElMessageBox.confirm(
      `确定删除数据库「${row.title}」吗？里面共有 ${row.row_count} 条数据，删除后会被移到 Notion 回收站。`,
      '删除确认',
      { confirmButtonText: '删除', cancelButtonText: '取消', type: 'warning' }
    )
  } catch {
    return
  }
  try {
    await api.deleteDatabase({ token: form.token.trim(), database_id: row.id })
    ElMessage.success('已删除')
    await refreshCandidates()
  } catch (e) {
    ElMessage.error(e.message || '删除失败')
  }
}

function openMergeDialog() {
  if (!canMerge.value) {
    ElMessage.warning('请至少选择两个同类型的库进行合并')
    return
  }
  mergeTargetId.value = selectedCandidates.value[0].id
  mergeDeleteSources.value = false
  mergeDialogVisible.value = true
}

async function doMerge() {
  const list = selectedCandidates.value
  const target = list.find((c) => c.id === mergeTargetId.value)
  if (!target) {
    ElMessage.warning('请选择目标库')
    return
  }
  const sources = list.filter((c) => c.id !== target.id).map((c) => c.id)
  merging.value = true
  try {
    const res = await api.mergeDatabases({
      token: form.token.trim(),
      kind: list[0].kind,
      target_id: target.id,
      source_ids: sources,
      delete_sources: mergeDeleteSources.value,
    })
    ElMessage.success(`合并完成，共复制 ${res.copied} 条数据`)
    mergeDialogVisible.value = false
    await refreshCandidates()
  } catch (e) {
    ElMessage.error(e.message || '合并失败')
  } finally {
    merging.value = false
  }
}

async function submit() {
  // 管理模式（重新绑定）允许留空：后端回退到已保存的 token / 页面
  if (!props.closable && (!form.token.trim() || !form.page_url.trim())) {
    error.value = '请填写 Token 和页面 URL'
    return
  }
  error.value = ''
  submitting.value = true
  try {
    const payload = {
      token: form.token.trim(),
      page_url: form.page_url.trim(),
      events_db_id: selectedIds.events || undefined,
      expenses_db_id: selectedIds.expenses || undefined,
      projects_db_id: selectedIds.projects || undefined,
      notes_db_id: selectedIds.notes || undefined,
    }
    await api.setup(payload)
    ElMessage.success('配置成功')
    emit('done')
    await notifySetupDone()
  } catch (e) {
    error.value = e.message
  } finally {
    submitting.value = false
  }
}
</script>

<style scoped>
.setup-page {
  min-height: 100vh;
  display: flex;
  justify-content: center;
  align-items: flex-start;
  padding: 24px;
  overflow-y: auto;
}

.setup-card {
  max-width: 720px;
  width: 100%;
}

.sub {
  color: #909399;
}

.setup-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.steps {
  line-height: 1.9;
  padding-left: 20px;
}

.err {
  margin-bottom: 18px;
}
</style>
