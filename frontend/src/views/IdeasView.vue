<template>
  <div>
    <!-- 快速输入 -->
    <el-card shadow="never" class="quick-card">
      <el-input
        v-model="draft"
        type="textarea"
        :rows="3"
        placeholder="记录一个想法…（Ctrl+Enter 收集）"
        @keydown.ctrl.enter.prevent="submit"
        @keydown.meta.enter.prevent="submit"
      />
      <div class="quick-actions">
        <el-select v-model="draftTag" style="width: 120px">
          <el-option v-for="t in IDEA_TAGS" :key="t" :label="t" :value="t" />
        </el-select>
        <el-button type="primary" :loading="creating" @click="submit">收集</el-button>
      </div>
    </el-card>

    <!-- 标签筛选 -->
    <div class="filter-chips">
      <el-check-tag :checked="filterTag === ''" @change="toggleTag('')">
        全部 {{ totalCount }}
      </el-check-tag>
      <el-check-tag
        v-for="t in IDEA_TAGS"
        :key="t"
        :checked="filterTag === t"
        @change="toggleTag(t)"
      >
        {{ t }} {{ tagCounts[t] || 0 }}
      </el-check-tag>
    </div>

    <!-- 卡片墙 -->
    <el-empty
      v-if="ideas.length === 0 && !loading"
      :description="filterTag ? `「${filterTag}」标签下还没有想法` : '还没有想法，写下第一个吧'"
      :image-size="80"
    />
    <div v-else class="wall" v-loading="loading">
      <div v-for="idea in ideas" :key="idea.id" class="idea-card" :class="{ pinned: idea.pinned }">
        <el-icon v-if="idea.pinned" class="pin-badge" color="#e6a23c"><Top /></el-icon>
        <div class="idea-content">{{ idea.content }}</div>
        <div class="idea-footer">
          <el-tag :type="ideaTagType(idea.tag)" size="small">{{ idea.tag || '其他' }}</el-tag>
          <span class="idea-time">{{ relTime(idea.created_ts) }}</span>
          <span class="idea-ops">
            <el-icon :title="idea.pinned ? '取消置顶' : '置顶'" @click="togglePin(idea)">
              <Top v-if="!idea.pinned" /><Bottom v-else />
            </el-icon>
            <el-icon title="编辑" @click="openEdit(idea)"><EditPen /></el-icon>
            <el-icon title="删除" class="op-danger" @click="removeIdea(idea)"><Delete /></el-icon>
          </span>
        </div>
      </div>
    </div>

    <!-- 编辑弹窗 -->
    <el-dialog v-model="editDialog" title="编辑想法" width="460px">
      <el-input v-model="editForm.content" type="textarea" :rows="4" />
      <el-select v-model="editForm.tag" style="width: 100%; margin-top: 12px">
        <el-option v-for="t in IDEA_TAGS" :key="t" :label="t" :value="t" />
      </el-select>
      <template #footer>
        <el-button @click="editDialog = false">取消</el-button>
        <el-button type="primary" :loading="saving" @click="saveEdit">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Bottom, Delete, EditPen, Top } from '@element-plus/icons-vue'
import { api } from '../api'
import { IDEA_TAGS, IDEA_TAG_TYPES, relTime } from '../utils'

const allIdeas = ref([])
const loading = ref(false)
const filterTag = ref('')

const draft = ref('')
const draftTag = ref('灵感')
const creating = ref(false)

const editDialog = ref(false)
const saving = ref(false)
const editForm = reactive({ id: null, content: '', tag: '灵感' })

function ideaTagType(tag) {
  return IDEA_TAG_TYPES[tag] || 'info'
}

// 一次拉全量，筛选与计数都在本地计算，保证筛选时各标签计数仍然准确
const ideas = computed(() =>
  filterTag.value ? allIdeas.value.filter((i) => (i.tag || '其他') === filterTag.value) : allIdeas.value,
)
const totalCount = computed(() => allIdeas.value.length)
const tagCounts = computed(() => {
  const map = {}
  for (const i of allIdeas.value) {
    const t = i.tag || '其他'
    map[t] = (map[t] || 0) + 1
  }
  return map
})

async function load() {
  loading.value = true
  try {
    allIdeas.value = await api.getIdeas()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loading.value = false
  }
}

function toggleTag(t) {
  filterTag.value = filterTag.value === t ? '' : t
}

async function submit() {
  const content = draft.value.trim()
  if (!content) {
    ElMessage.warning('先写点什么吧')
    return
  }
  creating.value = true
  try {
    await api.createIdea({ content, tag: draftTag.value })
    draft.value = ''
    ElMessage.success('已收集')
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    creating.value = false
  }
}

async function togglePin(idea) {
  try {
    await api.updateIdea(idea.id, { pinned: !idea.pinned })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

function openEdit(idea) {
  editForm.id = idea.id
  editForm.content = idea.content
  editForm.tag = IDEA_TAGS.includes(idea.tag) ? idea.tag : '其他'
  editDialog.value = true
}

async function saveEdit() {
  if (!editForm.content.trim()) {
    ElMessage.warning('内容不能为空')
    return
  }
  saving.value = true
  try {
    await api.updateIdea(editForm.id, { content: editForm.content.trim(), tag: editForm.tag })
    editDialog.value = false
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}

async function removeIdea(idea) {
  try {
    await ElMessageBox.confirm('删除这条想法？', '确认删除', { type: 'warning' })
  } catch {
    return
  }
  try {
    await api.deleteIdea(idea.id)
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.quick-card {
  margin-bottom: 14px;
}

.quick-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 10px;
}

.filter-chips {
  display: flex;
  gap: 10px;
  margin-bottom: 14px;
  flex-wrap: wrap;
}

.wall {
  columns: 3;
  column-gap: 14px;
}

@media (max-width: 900px) {
  .wall {
    columns: 2;
  }
}

@media (max-width: 600px) {
  .wall {
    columns: 1;
  }
}

.idea-card {
  position: relative;
  break-inside: avoid;
  margin-bottom: 14px;
  background: #fff;
  border: 1px solid #e4e7ed;
  border-radius: 10px;
  padding: 14px 16px;
}

.idea-card.pinned {
  background: #fdf6ec;
  border-color: #f3d19e;
}

.pin-badge {
  position: absolute;
  top: 10px;
  right: 12px;
}

.idea-content {
  white-space: pre-wrap;
  word-break: break-word;
  line-height: 1.7;
  font-size: 14px;
}

.idea-footer {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 10px;
}

.idea-time {
  font-size: 12px;
  color: #909399;
}

.idea-ops {
  display: none;
  margin-left: auto;
  gap: 8px;
  color: #909399;
}

.idea-card:hover .idea-ops {
  display: inline-flex;
}

.idea-ops .el-icon:hover {
  color: #409eff;
}

.idea-ops .op-danger:hover {
  color: #f56c6c;
}
</style>
