<template>
  <div class="nte" v-loading="loading">
    <div class="nte-header">
      <span class="nte-title">知识库</span>
      <el-input
        v-model="query"
        size="small"
        placeholder="搜索标题…"
        clearable
        class="nte-search"
      />
      <span class="nte-open" @click="openFull">打开完整知识库 →</span>
    </div>

    <!-- 空态 -->
      <div v-if="!loading && flatNodes.length === 0 && !query" class="ntl-empty">
        知识库还是空的
        <div class="ntl-empty-actions">
          <el-button size="small" @click="createNode('dir', null)">📁 新建目录</el-button>
          <el-button size="small" @click="createNode('doc', null)">📄 新建文档</el-button>
        </div>
      </div>

    <div v-else class="nte-body">
      <!-- 左侧：目录树 / 搜索结果（右键新建目录/文档） -->
      <div class="nte-tree" @contextmenu.prevent="onTreeContextMenu($event, null)">
        <template v-if="query">
          <div v-if="searchRows.length === 0" class="nte-no-match">无匹配</div>
          <div
            v-for="n in searchRows"
            :key="n.id"
            class="nte-node"
            :class="{ active: n.id === selectedId }"
            @click="onNodeClick(n)"
          >
            <span class="nte-icon">{{ n.kind === 'dir' ? '📁' : '📄' }}</span>
            <span class="nte-name" :title="n.path">{{ n.path }}</span>
          </div>
        </template>
        <template v-else>
          <div
            v-for="n in flatNodes"
            :key="n.id"
            class="nte-node"
            :class="{ active: n.id === selectedId, dir: n.kind === 'dir' }"
            :style="{ paddingLeft: 8 + n.level * 14 + 'px' }"
            @click="onNodeClick(n)"
            @contextmenu.prevent.stop="onTreeContextMenu($event, n)"
          >
            <span class="nte-arrow">{{ n.kind === 'dir' ? (expanded.has(n.id) ? '▼' : '▶') : '' }}</span>
            <span class="nte-icon">{{ n.kind === 'dir' ? '📁' : '📄' }}</span>
            <span class="nte-name" :title="n.title">{{ n.title }}</span>
          </div>
        </template>
      </div>

      <!-- 右键菜单：新建目录/文档 -->
      <Teleport to="body">
        <div
          v-if="ctx.show"
          class="nte-ctxmenu"
          :style="{ left: ctx.x + 'px', top: ctx.y + 'px' }"
          @click.stop
        >
          <div class="nte-ctx-title">{{ ctxHint }}</div>
          <div class="nte-ctx-item" @click="createNode('dir')">📁 新建目录</div>
          <div class="nte-ctx-item" @click="createNode('doc')">📄 新建文档</div>
        </div>
      </Teleport>

      <!-- 右侧：文档内容 -->
      <div class="nte-content" v-loading="docLoading">
        <template v-if="doc">
          <div class="nte-doc-header">
            <div class="nte-doc-title">{{ doc.title }}</div>
            <div class="nte-doc-ops">
              <el-button
                v-if="editing && dirty"
                size="small"
                type="primary"
                :loading="saving"
                @click="saveDoc"
              >保存</el-button>
              <el-button size="small" text @click="toggleEdit">
                {{ editing ? '预览' : '编辑' }}
              </el-button>
            </div>
          </div>
          <el-input
            v-if="editing"
            v-model="draft"
            type="textarea"
            class="nte-editor"
            :autosize="{ minRows: 12, maxRows: 24 }"
            placeholder="Markdown 内容（Ctrl+S 保存）"
            @keydown.ctrl.s.prevent="saveDoc"
          />
          <!-- eslint-disable-next-line vue/no-v-html -->
          <div v-else class="md-body" v-html="contentHtml" />
        </template>
        <div v-else class="nte-placeholder">选择左侧文档查看内容</div>
      </div>
    </div>
  </div>
</template>

<script setup>
import { computed, inject, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from '../api'

const tree = ref([])
const loading = ref(false)
const expanded = ref(new Set())
const selectedId = ref(null)
const doc = ref(null)
const docLoading = ref(false)

// 搜索
const query = ref('')

// 编辑
const editing = ref(false)
const draft = ref('')
const saving = ref(false)
const dirty = computed(() => doc.value && draft.value !== (doc.value.content_md || ''))

// 拍平可见节点（目录展开状态决定子节点是否可见），parentId 供右键新建定位
const flatNodes = computed(() => {
  const out = []
  const walk = (nodes, level, parentId) => {
    for (const n of nodes || []) {
      out.push({ id: n.id, kind: n.kind, title: n.title, level, parentId })
      if (n.kind === 'dir' && expanded.value.has(n.id) && n.children?.length) {
        walk(n.children, level + 1, n.id)
      }
    }
  }
  walk(tree.value, 0, null)
  return out
})

// ---- 右键菜单：新建目录/文档 ----
const ctx = reactive({ show: false, x: 0, y: 0, target: null })
const ctxHint = computed(() => {
  if (!ctx.target) return '在根目录下新建'
  return ctx.target.kind === 'dir' ? `在「${ctx.target.title}」下新建` : '在同级新建'
})

function onTreeContextMenu(e, node) {
  ctx.show = true
  ctx.target = node
  // 防止菜单超出视口
  ctx.x = Math.min(e.clientX, window.innerWidth - 160)
  ctx.y = Math.min(e.clientY, window.innerHeight - 130)
}

function closeCtx() {
  ctx.show = false
}

async function createNode(kind, target = ctx.target) {
  closeCtx()
  // 目标为目录 -> 建在其内；目标为文档 -> 建在其同级；空白处 -> 根目录
  const parentId = !target
    ? null
    : target.kind === 'dir'
      ? target.id
      : target.parentId
  const label = kind === 'dir' ? '目录' : '文档'
  let title
  try {
    const res = await ElMessageBox.prompt(`请输入${label}名称`, `新建${label}`, {
      inputPattern: /\S+/,
      inputErrorMessage: '名称不能为空',
    })
    title = res.value.trim()
  } catch {
    return // 取消
  }
  try {
    const created = await api.createNote({
      parent_id: parentId,
      kind,
      title,
      ...(kind === 'doc' ? { content_md: `# ${title}\n\n` } : {}),
    })
    await load()
    // 展开父目录；新文档直接选中
    if (parentId) expanded.value = new Set([...expanded.value, parentId])
    if (created.kind === 'doc') selectDoc(created.id)
    ElMessage.success(`${label}「${title}」已创建`)
  } catch (e) {
    ElMessage.error(e.message)
  }
}

// 搜索：拍平全部节点（含路径），按标题匹配
const searchRows = computed(() => {
  const q = query.value.trim().toLowerCase()
  if (!q) return []
  const out = []
  const walk = (nodes, prefix) => {
    for (const n of nodes || []) {
      const path = prefix ? `${prefix} / ${n.title}` : n.title
      if (n.title.toLowerCase().includes(q)) {
        out.push({ id: n.id, kind: n.kind, title: n.title, path })
      }
      if (n.children?.length) walk(n.children, path)
    }
  }
  walk(tree.value, '')
  return out
})

const contentHtml = computed(() =>
  doc.value ? DOMPurify.sanitize(marked.parse(doc.value.content_md || '')) : ''
)

const openPanel = inject('openPanel', null)
function openFull() {
  if (openPanel) openPanel('notes')
}

function onNodeClick(n) {
  if (n.kind === 'dir') {
    const s = new Set(expanded.value)
    if (s.has(n.id)) s.delete(n.id)
    else s.add(n.id)
    expanded.value = s
  } else {
    selectDoc(n.id)
  }
}

async function selectDoc(id) {
  await flushAutosave() // 切换文档前保存上一篇的未落库修改
  selectedId.value = id
  editing.value = false
  docLoading.value = true
  try {
    doc.value = await api.getNote(id)
    draft.value = doc.value.content_md || ''
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    docLoading.value = false
  }
}

function toggleEdit() {
  if (!editing.value) {
    draft.value = doc.value?.content_md || ''
  }
  editing.value = !editing.value
}

async function saveDoc(auto = false) {
  if (!doc.value || !dirty.value) return
  saving.value = true
  try {
    const updated = await api.updateNote(doc.value.id, { content_md: draft.value })
    doc.value = { ...doc.value, content_md: updated.content_md ?? draft.value }
    if (!auto) ElMessage.success('已保存')
  } catch (e) {
    ElMessage.error('保存失败: ' + e.message)
  } finally {
    saving.value = false
  }
}

// 自动保存：编辑中停笔 3 秒落库；切换文档/卸载前先冲刷
let autosaveTimer = null
watch(draft, () => {
  if (!editing.value || !dirty.value) return
  clearTimeout(autosaveTimer)
  autosaveTimer = setTimeout(() => saveDoc(true), 3000)
})

async function flushAutosave() {
  clearTimeout(autosaveTimer)
  if (editing.value && dirty.value) await saveDoc(true)
}

// 找到第一个文档（深度优先）
function firstDoc(nodes) {
  for (const n of nodes || []) {
    if (n.kind === 'doc') return n
    const d = firstDoc(n.children)
    if (d) return d
  }
  return null
}
// 收集所有目录 id（默认全部展开）
function allDirIds(nodes, out = []) {
  for (const n of nodes || []) {
    if (n.kind === 'dir') {
      out.push(n.id)
      allDirIds(n.children, out)
    }
  }
  return out
}

async function load() {
  loading.value = true
  try {
    tree.value = await api.getNotesTree()
    expanded.value = new Set(allDirIds(tree.value))
    const first = firstDoc(tree.value)
    if (first) selectDoc(first.id)
  } catch (e) {
    ElMessage.error(`加载知识库失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  load()
  document.addEventListener('click', closeCtx)
})
onUnmounted(() => {
  document.removeEventListener('click', closeCtx)
  clearTimeout(autosaveTimer)
  if (editing.value && dirty.value) saveDoc(true) // 卸载前兜底保存
})
</script>

<style scoped>
.nte {
  min-height: 80px;
}
.nte-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  margin-bottom: 8px;
}
.nte-title {
  font-size: 13px;
  font-weight: 600;
  color: #303133;
  flex-shrink: 0;
}
.nte-search {
  flex: 1;
  max-width: 180px;
}
.nte-doc-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
  padding-bottom: 6px;
  border-bottom: 1px solid #ebeef5;
}
.nte-doc-title {
  font-size: 14px;
  font-weight: 600;
  color: #303133;
}
.nte-doc-ops {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}
.nte-editor :deep(textarea) {
  font-family: 'SF Mono', Consolas, monospace;
  font-size: 12px;
  line-height: 1.6;
}
.nte-no-match {
  color: #c0c4cc;
  font-size: 12px;
  text-align: center;
  padding: 20px 0;
}
.nte-open {
  font-size: 12px;
  color: #409eff;
  cursor: pointer;
  flex-shrink: 0;
}
.nte-open:hover {
  text-decoration: underline;
}
.ntl-empty {
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
  color: #909399;
  font-size: 13px;
  text-align: center;
  padding: 24px 0;
}
.ntl-empty-actions {
  margin-top: 12px;
  display: flex;
  gap: 8px;
  justify-content: center;
}
.nte-body {
  display: flex;
  gap: 8px;
  height: calc(70vh - 52px);
  min-height: 360px;
}
.nte-tree {
  width: 180px;
  flex-shrink: 0;
  overflow-y: auto;
  border-right: 1px solid #ebeef5;
  padding-right: 4px;
}
.nte-node {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 5px 6px 5px 8px;
  border-radius: 5px;
  cursor: pointer;
  font-size: 12px;
  color: #303133;
}
.nte-node:hover {
  background: #f5f7fa;
}
.nte-node.active {
  background: #ecf5ff;
  color: #409eff;
}
.nte-arrow {
  width: 12px;
  flex-shrink: 0;
  font-size: 8px;
  color: #909399;
}
.nte-icon {
  flex-shrink: 0;
  font-size: 12px;
}
.nte-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.nte-content {
  flex: 1;
  overflow-y: auto;
  min-width: 0;
}
.nte-placeholder {
  color: #c0c4cc;
  font-size: 12px;
  text-align: center;
  margin-top: 60px;
}
/* Markdown 内容基础样式 */
.md-body {
  font-size: 13px;
  line-height: 1.6;
  color: #303133;
}
.md-body :deep(h1),
.md-body :deep(h2),
.md-body :deep(h3) {
  margin: 10px 0 6px;
  line-height: 1.3;
}
.md-body :deep(h1) { font-size: 17px; }
.md-body :deep(h2) { font-size: 15px; }
.md-body :deep(h3) { font-size: 14px; }
.md-body :deep(p) { margin: 6px 0; }
.md-body :deep(ul),
.md-body :deep(ol) { margin: 6px 0; padding-left: 20px; }
.md-body :deep(code) {
  background: #f5f7fa;
  padding: 1px 5px;
  border-radius: 4px;
  font-size: 12px;
}
.md-body :deep(pre) {
  background: #f5f7fa;
  padding: 10px;
  border-radius: 6px;
  overflow-x: auto;
}
.md-body :deep(pre code) {
  background: none;
  padding: 0;
}
.md-body :deep(blockquote) {
  margin: 6px 0;
  padding: 4px 10px;
  border-left: 3px solid #dcdfe6;
  color: #909399;
}
.md-body :deep(img) { max-width: 100%; }
.md-body :deep(table) { border-collapse: collapse; }
.md-body :deep(th),
.md-body :deep(td) { border: 1px solid #dcdfe6; padding: 4px 8px; }
</style>

<!-- 右键菜单 Teleport 到 body，样式不能 scoped -->
<style>
.nte-ctxmenu {
  position: fixed;
  z-index: 3000;
  min-width: 140px;
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  box-shadow: 0 4px 16px rgba(0,0,0,0.15);
  padding: 4px;
}
.nte-ctx-title {
  font-size: 11px;
  color: #909399;
  padding: 4px 10px;
  border-bottom: 1px solid #f2f3f5;
  margin-bottom: 2px;
}
.nte-ctx-item {
  font-size: 13px;
  color: #303133;
  padding: 6px 10px;
  border-radius: 5px;
  cursor: pointer;
}
.nte-ctx-item:hover {
  background: #ecf5ff;
  color: #409eff;
}
</style>
