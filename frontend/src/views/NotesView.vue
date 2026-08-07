<template>
  <div>
    <!-- ============ 总览模式 ============ -->
    <template v-if="viewMode === 'overview'">
      <div v-if="rootDirs.length === 0 && rootDocs.length === 0" class="overview-empty" v-loading="loadingTree">
        <el-icon :size="56" color="#c0c4cc"><Notebook /></el-icon>
        <p class="empty-text">还没有知识库</p>
        <el-button type="primary" @click="openCreateBook">创建你的第一个知识库</el-button>
      </div>

      <template v-else>
        <div class="book-grid" v-loading="loadingTree">
          <div v-for="b in rootDirs" :key="b.id" class="book-card" @click="enterBook(b)">
            <div class="book-top">
              <el-icon :size="22" color="#e6a23c"><Notebook /></el-icon>
              <span class="book-name">{{ b.title }}</span>
              <span class="book-ops" @click.stop>
                <el-icon title="重命名" @click="openRename(b)"><EditPen /></el-icon>
                <el-icon title="删除" class="op-danger" @click="removeNode(b)"><Delete /></el-icon>
              </span>
            </div>
            <div class="book-stats">
              {{ statsOf(b).docs }} 个文档 / {{ statsOf(b).dirs }} 个子目录
            </div>
            <div v-if="statsOf(b).latestTs" class="book-updated">
              最近更新 {{ fmtTs(statsOf(b).latestTs) }}
            </div>
          </div>
          <div class="book-card create-card" @click="openCreateBook">
            <span class="create-plus">＋</span>
            <span>创建知识库</span>
          </div>
        </div>

        <!-- 根级散文档（历史数据） -->
        <div v-if="rootDocs.length > 0" class="unfiled">
          <h4>未归档文档</h4>
          <div class="dir-cards">
            <div v-for="d in rootDocs" :key="d.id" class="dir-card" @click="enterDoc(d)">
              <el-icon><Document /></el-icon>
              <span>{{ d.title }}</span>
            </div>
          </div>
        </div>
      </template>
    </template>

    <!-- ============ 编辑器模式 ============ -->
    <template v-else>
      <div class="editor-topbar">
        <el-button size="small" @click="backToOverview">← 返回</el-button>
        <span class="editor-book-name">{{ book?.title || '（已删除）' }}</span>
      </div>

      <div class="notes-layout">
        <!-- 左侧：当前知识库子树 -->
        <el-card shadow="never" class="tree-panel">
          <div v-if="book && book.kind === 'dir'" class="tree-actions">
            <el-button size="small" @click="openCreate('dir', book)">新建目录</el-button>
            <el-button size="small" @click="openCreate('doc', book)">新建文档</el-button>
          </div>
          <el-tree
            ref="treeRef"
            :data="treeData"
            node-key="id"
            :props="{ label: 'title', children: 'children' }"
            :expand-on-click-node="false"
            default-expand-all
            :draggable="!!book && book.kind === 'dir'"
            :allow-drop="allowDrop"
            highlight-current
            empty-text="空知识库，点上方按钮新建"
            v-loading="loadingTree"
            @node-click="onNodeClick"
            @node-drop="onNodeDrop"
          >
            <template #default="{ data }">
              <span class="tree-node">
                <el-icon class="node-icon">
                  <Folder v-if="data.kind === 'dir'" />
                  <Document v-else />
                </el-icon>
                <span class="node-title">{{ data.title }}</span>
                <span class="node-ops" @click.stop>
                  <template v-if="data.kind === 'dir'">
                    <el-icon title="新建子目录" @click="openCreate('dir', data)"><FolderAdd /></el-icon>
                    <el-icon title="新建子文档" @click="openCreate('doc', data)"><DocumentAdd /></el-icon>
                  </template>
                  <el-icon title="重命名" @click="openRename(data)"><EditPen /></el-icon>
                  <el-icon title="删除" class="op-danger" @click="removeNode(data)"><Delete /></el-icon>
                </span>
              </span>
            </template>
          </el-tree>
        </el-card>

        <!-- 右侧：内容区 -->
        <el-card shadow="never" class="content-panel">
          <el-empty v-if="!current" description="从左侧选择一篇文档或目录" :image-size="90" />

          <!-- 文档 -->
          <template v-else-if="current.kind === 'doc'">
            <div class="doc-header">
              <span class="doc-title">{{ current.title }}</span>
              <span class="doc-updated">更新于 {{ fmtTs(current.updated_ts) }}</span>
              <el-radio-group v-model="editMode" size="small">
                <el-radio-button :value="false">预览</el-radio-button>
                <el-radio-button :value="true">编辑</el-radio-button>
              </el-radio-group>
              <el-button
                v-if="editMode"
                size="small"
                type="primary"
                :loading="saving"
                @click="saveContent"
              >保存</el-button>
            </div>
            <div v-if="editMode" class="editor-wrap" v-loading="loadingDoc">
              <textarea
                v-model="draft"
                class="editor"
                placeholder="支持 Markdown，Ctrl+S 保存"
                @blur="saveContent"
                @keydown.ctrl.s.prevent="saveContent"
                @keydown.meta.s.prevent="saveContent"
              />
            </div>
            <div v-else class="markdown-body" v-loading="loadingDoc" v-html="rendered" />
          </template>

          <!-- 目录 -->
          <template v-else>
            <div class="doc-header">
              <span class="doc-title">📁 {{ current.title }}</span>
            </div>
            <el-empty v-if="!current.children || current.children.length === 0" description="空目录" :image-size="70" />
            <div v-else class="dir-cards">
              <div
                v-for="c in current.children"
                :key="c.id"
                class="dir-card"
                @click="selectNode(c.id)"
              >
                <el-icon><Folder v-if="c.kind === 'dir'" /><Document v-else /></el-icon>
                <span>{{ c.title }}</span>
              </div>
            </div>
          </template>
        </el-card>
      </div>
    </template>

    <!-- 新建 / 重命名对话框 -->
    <el-dialog v-model="nameDialog" :title="nameDialogTitle" width="420px">
      <el-input
        v-model="nameInput"
        :placeholder="nameKind === 'dir' ? '目录名称' : '文档标题'"
        @keyup.enter="submitName"
      />
      <template #footer>
        <el-button @click="nameDialog = false">取消</el-button>
        <el-button type="primary" :loading="submitting" @click="submitName">确定</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import {
  Delete,
  Document,
  DocumentAdd,
  EditPen,
  Folder,
  FolderAdd,
  Notebook,
} from '@element-plus/icons-vue'
import { marked } from 'marked'
import DOMPurify from 'dompurify'
import { api } from '../api'
import { fmtTs } from '../utils'

// ---------- 模式与树 ----------

const viewMode = ref('overview') // overview | editor
const currentBookId = ref(null)
const tree = ref([])
const loadingTree = ref(false)
const treeRef = ref(null)

const current = ref(null) // 选中的树节点
const doc = ref(null) // 文档完整对象（含 content_md）
const loadingDoc = ref(false)
const editMode = ref(false)
const draft = ref('')
const saving = ref(false)

// 新建 / 重命名
const nameDialog = ref(false)
const nameInput = ref('')
const submitting = ref(false)
const nameMode = ref('create') // create | rename
const nameKind = ref('doc')
const nameParent = ref(null) // 父节点或 null（根）
const nameTarget = ref(null)

const rootDirs = computed(() => tree.value.filter((n) => n.kind === 'dir'))
const rootDocs = computed(() => tree.value.filter((n) => n.kind === 'doc'))

const book = computed(() =>
  currentBookId.value ? findNode(tree.value, currentBookId.value) : null
)

// 编辑器模式左树数据：库的 children；未归档散文档则只显示该文档自身
const treeData = computed(() => {
  if (!book.value) return []
  return book.value.kind === 'dir' ? book.value.children || [] : [book.value]
})

const nameDialogTitle = computed(() =>
  nameMode.value === 'rename' ? '重命名' : nameKind.value === 'dir' ? '目录名称' : '文档标题'
)

const rendered = computed(() => {
  if (!doc.value) return ''
  return DOMPurify.sanitize(marked.parse(doc.value.content_md || ''))
})

function findNode(nodes, id) {
  for (const n of nodes) {
    if (n.id === id) return n
    const hit = findNode(n.children || [], id)
    if (hit) return hit
  }
  return null
}

// 递归统计：文档数 / 子目录数 / 最近更新时间
function statsOf(node) {
  let docs = 0
  let dirs = 0
  let latestTs = 0
  const walk = (n) => {
    if (n.updated_ts && n.updated_ts > latestTs) latestTs = n.updated_ts
    for (const c of n.children || []) {
      if (c.kind === 'dir') dirs += 1
      else docs += 1
      walk(c)
    }
  }
  walk(node)
  return { docs, dirs, latestTs }
}

async function loadTree() {
  const selectedId = current.value?.id
  loadingTree.value = true
  try {
    tree.value = await api.getNotesTree()
    // 保持当前模式与选中
    if (viewMode.value === 'editor' && !book.value) {
      backToOverview()
      return
    }
    if (selectedId) {
      const node = findNode(tree.value, selectedId)
      if (node) {
        current.value = node
        treeRef.value?.setCurrentKey(selectedId)
      } else {
        current.value = null
        doc.value = null
      }
    }
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loadingTree.value = false
  }
}

// ---------- 模式切换 ----------

function enterBook(b) {
  viewMode.value = 'editor'
  currentBookId.value = b.id
  current.value = null
  doc.value = null
}

function enterDoc(d) {
  viewMode.value = 'editor'
  currentBookId.value = d.id
  onNodeClick(d)
}

function backToOverview() {
  viewMode.value = 'overview'
  currentBookId.value = null
  current.value = null
  doc.value = null
}

// ---------- 选中 ----------

async function onNodeClick(data) {
  current.value = data
  if (data.kind === 'doc') {
    editMode.value = false
    loadingDoc.value = true
    try {
      doc.value = await api.getNote(data.id)
      draft.value = doc.value.content_md || ''
    } catch (e) {
      ElMessage.error(e.message)
    } finally {
      loadingDoc.value = false
    }
  }
}

function selectNode(id) {
  const node = findNode(tree.value, id)
  if (node) {
    treeRef.value?.setCurrentKey(id)
    onNodeClick(node)
  }
}

// ---------- 新建 / 重命名 ----------

function openCreateBook() {
  nameMode.value = 'create'
  nameKind.value = 'dir'
  nameParent.value = null
  nameTarget.value = null
  nameInput.value = ''
  nameDialog.value = true
}

function openCreate(kind, parent) {
  nameMode.value = 'create'
  nameKind.value = kind
  nameParent.value = parent
  nameTarget.value = null
  nameInput.value = ''
  nameDialog.value = true
}

function openRename(node) {
  nameMode.value = 'rename'
  nameTarget.value = node
  nameInput.value = node.title
  nameDialog.value = true
}

async function submitName() {
  const title = nameInput.value.trim()
  if (!title) {
    ElMessage.warning('请输入名称')
    return
  }
  submitting.value = true
  try {
    if (nameMode.value === 'rename') {
      await api.updateNote(nameTarget.value.id, { title })
      nameDialog.value = false
      await loadTree()
    } else {
      const created = await api.createNote({
        parent_id: nameParent.value ? nameParent.value.id : null,
        kind: nameKind.value,
        title,
        ...(nameKind.value === 'doc' ? { content_md: `# ${title}\n\n` } : {}),
      })
      nameDialog.value = false
      await loadTree()
      if (created.kind === 'doc' && viewMode.value === 'editor') selectNode(created.id)
    }
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    submitting.value = false
  }
}

// ---------- 删除 ----------

async function removeNode(node) {
  try {
    await ElMessageBox.confirm(
      node.kind === 'dir'
        ? `删除目录「${node.title}」？其中所有子目录和文档都会被一并删除。`
        : `删除文档「${node.title}」？`,
      '确认删除',
      { type: 'warning' }
    )
  } catch {
    return
  }
  try {
    await api.deleteNote(node.id)
    if (current.value?.id === node.id) {
      current.value = null
      doc.value = null
    }
    if (node.id === currentBookId.value) backToOverview()
    await loadTree()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

// ---------- 拖拽移动 ----------

function allowDrop(draggingNode, dropNode, type) {
  // 不允许拖到文档的内部
  if (type === 'inner' && dropNode.data.kind !== 'dir') return false
  // 不允许把目录拖进自己或自己的子孙
  if (isDescendant(draggingNode.data, dropNode.data.id)) return false
  return true
}

function isDescendant(node, id) {
  if (node.id === id) return true
  return (node.children || []).some((c) => isDescendant(c, id))
}

async function onNodeDrop(draggingNode, dropNode, dropType) {
  // 注意：el-tree 不会更新原始 data.parent_id，需用 Node 层级判断父节点；
  // 此处展示的是某个知识库的子树，因此 level 1 落点的父节点是库本身
  const rootParentId = book.value && book.value.kind === 'dir' ? book.value.id : null
  const newParentId =
    dropType === 'inner'
      ? dropNode.data.id
      : dropNode.level > 1
        ? dropNode.parent.data.id
        : rootParentId
  try {
    await api.updateNote(draggingNode.data.id, { parent_id: newParentId })
    await loadTree()
  } catch (e) {
    ElMessage.error(`移动失败：${e.message}`)
    await loadTree() // 后端兜底拒绝时恢复原状
  }
}

// ---------- 保存内容 ----------

async function saveContent() {
  if (!doc.value || saving.value) return
  if (draft.value === (doc.value.content_md || '')) return
  saving.value = true
  try {
    const updated = await api.updateNote(doc.value.id, { content_md: draft.value })
    doc.value = { ...doc.value, ...updated }
    ElMessage.success('已保存')
  } catch (e) {
    ElMessage.error(`保存失败：${e.message}`)
  } finally {
    saving.value = false
  }
}

onMounted(loadTree)
</script>

<style scoped>
/* ---------- 总览 ---------- */

.overview-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  padding: 80px 0;
  background: #fff;
  border-radius: 8px;
}

.empty-text {
  color: #909399;
  margin: 0;
}

.book-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 14px;
}

.book-card {
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 10px;
  padding: 16px;
  cursor: pointer;
  transition: border-color 0.15s, box-shadow 0.15s;
}

.book-card:hover {
  border-color: #409eff;
  box-shadow: 0 2px 8px rgba(64, 158, 255, 0.15);
}

.book-top {
  display: flex;
  align-items: center;
  gap: 8px;
}

.book-name {
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.book-ops {
  display: none;
  margin-left: auto;
  gap: 8px;
  color: #909399;
}

.book-card:hover .book-ops {
  display: inline-flex;
}

.book-ops .el-icon:hover {
  color: #409eff;
}

.book-ops .op-danger:hover {
  color: #f56c6c;
}

.book-stats {
  margin-top: 10px;
  font-size: 13px;
  color: #606266;
}

.book-updated {
  margin-top: 4px;
  font-size: 12px;
  color: #909399;
}

.create-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  min-height: 110px;
  border-style: dashed;
  color: #909399;
}

.create-card:hover {
  color: #409eff;
}

.create-plus {
  font-size: 26px;
  line-height: 1;
}

.unfiled {
  margin-top: 24px;
}

/* ---------- 编辑器 ---------- */

.editor-topbar {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 14px;
}

.editor-book-name {
  font-size: 17px;
  font-weight: 600;
}

.notes-layout {
  display: flex;
  gap: 16px;
  align-items: flex-start;
}

.tree-panel {
  flex: 0 0 280px;
  max-height: 80vh;
  overflow: auto;
}

.tree-actions {
  display: flex;
  gap: 8px;
  margin-bottom: 10px;
}

.tree-node {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  overflow: hidden;
}

.node-icon {
  flex: none;
  color: #909399;
}

.node-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.node-ops {
  display: none;
  margin-left: auto;
  gap: 6px;
  color: #909399;
}

.tree-node:hover .node-ops {
  display: inline-flex;
}

.node-ops .el-icon:hover {
  color: #409eff;
}

.node-ops .op-danger:hover {
  color: #f56c6c;
}

.content-panel {
  flex: 1;
  min-height: 560px;
}

.doc-header {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-bottom: 14px;
  flex-wrap: wrap;
}

.doc-title {
  font-size: 17px;
  font-weight: 600;
}

.doc-updated {
  color: #909399;
  font-size: 12px;
  margin-right: auto;
}

.editor-wrap {
  height: 58vh;
}

.editor {
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  border: 1px solid #dcdfe6;
  border-radius: 6px;
  padding: 12px;
  font-family: 'SF Mono', Menlo, Consolas, 'Courier New', monospace;
  font-size: 14px;
  line-height: 1.7;
  resize: none;
  outline: none;
}

.editor:focus {
  border-color: #409eff;
}

.markdown-body {
  line-height: 1.8;
  font-size: 14px;
  word-break: break-word;
}

.markdown-body :deep(h1),
.markdown-body :deep(h2),
.markdown-body :deep(h3) {
  margin: 0.8em 0 0.4em;
}

.markdown-body :deep(pre) {
  background: #f5f6fa;
  padding: 12px;
  border-radius: 6px;
  overflow-x: auto;
}

.markdown-body :deep(code) {
  font-family: 'SF Mono', Menlo, Consolas, monospace;
  font-size: 13px;
}

.markdown-body :deep(blockquote) {
  margin: 0.6em 0;
  padding-left: 12px;
  border-left: 3px solid #dcdfe6;
  color: #606266;
}

.markdown-body :deep(img) {
  max-width: 100%;
}

.markdown-body :deep(table) {
  border-collapse: collapse;
}

.markdown-body :deep(th),
.markdown-body :deep(td) {
  border: 1px solid #e4e7ed;
  padding: 4px 10px;
}

.dir-cards {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
  gap: 10px;
}

.dir-card {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px 14px;
  border: 1px solid #e4e7ed;
  border-radius: 8px;
  cursor: pointer;
  color: #606266;
  background: #fff;
}

.dir-card:hover {
  border-color: #409eff;
  color: #409eff;
  background: #ecf5ff;
}
</style>
