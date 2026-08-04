<template>
  <div class="idl" v-loading="loading">
    <!-- 快速收集（常驻） -->
    <div class="idl-add">
      <el-input
        v-model="draft"
        size="small"
        placeholder="记录一个想法…（Ctrl+Enter 收集）"
        @keydown.ctrl.enter="collect"
      />
      <el-select v-model="draftTag" size="small" style="width: 76px">
        <el-option v-for="t in IDEA_TAGS" :key="t" :label="t" :value="t" />
      </el-select>
      <el-button size="small" type="primary" :loading="adding" @click="collect">收集</el-button>
    </div>

    <div class="idl-header">
      <span class="idl-count">{{ scope === 'today' ? '今天' : '' }}共 {{ rows.length }} 条</span>
      <span class="idl-scope">
        <span :class="{ on: scope === 'today' }" @click="scope = 'today'">今天</span>
        <span :class="{ on: scope === 'recent' }" @click="scope = 'recent'">最近</span>
      </span>
      <span class="idl-open" @click="openFull">打开好想法 →</span>
    </div>

    <!-- 空态 -->
    <div v-if="!loading && rows.length === 0" class="idl-empty">
      {{ scope === 'today' ? '今天还没有想法，记一个吧' : '还没有想法，记一个吧' }}
    </div>

    <!-- 最近想法 -->
    <div v-for="idea in rows" :key="idea.id" class="idl-row" :class="{ pinned: idea.pinned }">
      <span
        class="idl-pin"
        :title="idea.pinned ? '取消置顶' : '置顶'"
        @click="togglePin(idea)"
      >{{ idea.pinned ? '📌' : '📍' }}</span>
      <span class="idl-content" :title="idea.content">{{ idea.content }}</span>
      <!-- 标签：点击可修改 -->
      <el-dropdown trigger="click" @command="(t) => setTag(idea, t)">
        <span class="idl-tag clickable" :style="{ background: tagColor(idea.tag) }">{{ idea.tag }}</span>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item v-for="t in IDEA_TAGS" :key="t" :command="t">
              <span class="idl-tag" :style="{ background: tagColor(t) }">{{ t }}</span>
            </el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
      <span class="idl-time">{{ relTime(idea.created_ts) }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed, inject, onMounted, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'
import { IDEA_TAGS, relTime } from '../utils'

const TAG_COLORS = {
  灵感: '#e6a23c',
  待办: '#409eff',
  读书: '#67c23a',
  问题: '#f56c6c',
  其他: '#909399',
}
function tagColor(t) {
  return TAG_COLORS[t] || '#909399'
}

const MAX_ROWS = 15
const ideas = ref([])
const loading = ref(false)
const draft = ref('')
const draftTag = ref('灵感')
const adding = ref(false)

// 范围：默认只显示今天的，可切「最近」（置顶在前，最近 15 条）
const scope = ref('today')

const rows = computed(() => {
  const sorted = ideas.value
    .slice()
    .sort((a, b) => (b.pinned - a.pinned) || b.created_ts - a.created_ts)
  if (scope.value === 'today') {
    return sorted.filter((i) => dayjs.unix(i.created_ts).isSame(dayjs(), 'day'))
  }
  return sorted.slice(0, MAX_ROWS)
})

const openPanel = inject('openPanel', null)
function openFull() {
  if (openPanel) openPanel('ideas')
}

async function load() {
  loading.value = true
  try {
    ideas.value = await api.getIdeas()
  } catch (e) {
    ElMessage.error(`加载想法失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

async function collect() {
  const content = draft.value.trim()
  if (!content) return
  adding.value = true
  try {
    await api.createIdea({ content, tag: draftTag.value })
    draft.value = ''
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adding.value = false
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

async function setTag(idea, tag) {
  if (idea.tag === tag) return
  try {
    await api.updateIdea(idea.id, { tag })
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(load)
</script>

<style scoped>
.idl {
  min-height: 80px;
}
.idl-add {
  display: flex;
  gap: 6px;
  margin-bottom: 8px;
}
.idl-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}
.idl-count {
  font-size: 12px;
  color: #909399;
  flex: 1;
}
.idl-scope {
  display: flex;
  gap: 8px;
  font-size: 12px;
}
.idl-scope span {
  cursor: pointer;
  color: #c0c4cc;
}
.idl-scope span.on {
  color: #faad14;
  font-weight: 600;
}
.idl-open {
  font-size: 12px;
  color: #409eff;
  cursor: pointer;
}
.idl-open:hover {
  text-decoration: underline;
}
.idl-empty {
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
  color: #909399;
  font-size: 13px;
  text-align: center;
  padding: 24px 0;
}
.idl-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  margin-bottom: 6px;
  background: #fff;
  border: 1px solid #ebeef5;
  border-left: 3px solid #faad14;
  border-radius: 6px;
}
.idl-row.pinned {
  border-left-color: #e6a23c;
  background: #fffbe6;
}
.idl-pin {
  flex-shrink: 0;
  font-size: 12px;
  cursor: pointer;
}
.idl-content {
  flex: 1;
  font-size: 13px;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.idl-tag {
  flex-shrink: 0;
  color: #fff;
  font-size: 10px;
  line-height: 1;
  padding: 3px 6px;
  border-radius: 4px;
}
.idl-tag.clickable {
  cursor: pointer;
}
.idl-tag.clickable:hover {
  filter: brightness(1.1);
}
.idl-time {
  flex-shrink: 0;
  font-size: 11px;
  color: #c0c4cc;
}
</style>
