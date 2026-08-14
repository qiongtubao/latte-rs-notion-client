<template>
  <div class="mel" v-loading="loading">
    <!-- 汇总行 + 范围切换 -->
    <div class="mel-summary">
      <span>今日 <b>¥{{ todayTotal.toFixed(2) }}</b></span>
      <span>本周 <b>¥{{ (summary.week || 0).toFixed(2) }}</b></span>
      <span>本月 <b>¥{{ (summary.month || 0).toFixed(2) }}</b></span>
      <span class="mel-scope">
        <span :class="{ on: scope === 'today' }" @click="scope = 'today'">今天</span>
        <span :class="{ on: scope === 'recent' }" @click="scope = 'recent'">最近</span>
      </span>
    </div>

    <!-- 内联记一笔：点子按钮「记一笔」出现；dock 宽度有限，分两行排布 -->
    <div v-if="showAdd" class="mel-add">
      <el-input
        ref="addInputRef"
        v-model="newItem"
        size="small"
        placeholder="事项"
        @keyup.enter="quickAdd"
        @keyup.esc="showAdd = false"
      />
      <div class="mel-addrow">
        <el-input-number
          v-model="newAmount"
          size="small"
          :min="0"
          :precision="2"
          :step="5"
          controls-position="right"
          style="flex: 1; min-width: 0"
          @keyup.enter="quickAdd"
        />
        <el-select v-model="newCategory" size="small" style="width: 96px; flex-shrink: 0">
          <el-option v-for="c in CATEGORIES" :key="c" :label="c" :value="c" />
        </el-select>
        <el-button size="small" type="primary" :loading="adding" @click="quickAdd">记</el-button>
      </div>
    </div>

    <!-- 空态 -->
    <div v-if="!loading && rows.length === 0" class="mel-empty">
      {{ scope === 'today' ? '今天还没有消费' : '还没有消费记录' }}
    </div>

    <!-- 消费列表 -->
    <div v-for="e in rows" :key="e.id" class="mel-row">
      <!-- 分类标签：点击可修改 -->
      <el-dropdown trigger="click" @command="(c) => setCategory(e, c)">
        <span class="mel-cat clickable">{{ e.category }}</span>
        <template #dropdown>
          <el-dropdown-menu>
            <el-dropdown-item v-for="c in CATEGORIES" :key="c" :command="c">{{ c }}</el-dropdown-item>
          </el-dropdown-menu>
        </template>
      </el-dropdown>
      <span class="mel-item" :title="e.item">{{ e.item }}</span>
      <span class="mel-time">{{ fmtTs(e.ts) }}</span>
      <span class="mel-amount">¥{{ (e.amount_cents / 100).toFixed(2) }}</span>
    </div>
  </div>
</template>

<script setup>
import { computed, inject, nextTick, onMounted, ref, watch } from 'vue'
import dayjs from 'dayjs'
import { ElMessage } from 'element-plus'
import { api } from '../api'

const CATEGORIES = ['餐饮', '交通', '购物', '娱乐', '医疗', '教育', '其他']
const MAX_ROWS = 20

const expenses = ref([])
const summary = ref({ week: 0, month: 0, year: 0 })
const loading = ref(false)

// 内联记一笔
const showAdd = ref(false)
const newItem = ref('')
const newAmount = ref(0)
const newCategory = ref('餐饮')
const adding = ref(false)
const addInputRef = ref(null)

// 范围：默认只显示今天的，可切「最近」（最近 20 条）
const scope = ref('today')

const isToday = (e) => dayjs.unix(e.ts).isSame(dayjs(), 'day')

// 消费列表：默认今天，「最近」为最近 20 条（按时间倒序）
const rows = computed(() => {
  const list = expenses.value.slice().sort((a, b) => b.ts - a.ts)
  if (scope.value === 'today') return list.filter(isToday)
  return list.slice(0, MAX_ROWS)
})
const todayTotal = computed(() =>
  expenses.value.filter(isToday).reduce((s, e) => s + e.amount_cents, 0) / 100
)

function fmtTs(ts) {
  const d = dayjs.unix(ts)
  return d.isSame(dayjs(), 'day') ? d.format('HH:mm') : d.format('MM-DD')
}

// 修改分类
async function setCategory(e, category) {
  if (e.category === category) return
  try {
    await api.updateExpense(e.id, { category })
    await load()
  } catch (err) {
    ElMessage.error(err.message)
  }
}

async function load() {
  loading.value = true
  try {
    const [es, s] = await Promise.all([api.getExpenses(), api.getExpenseSummary()])
    expenses.value = es
    summary.value = s
  } catch (e) {
    ElMessage.error(`加载消费失败：${e.message}`)
  } finally {
    loading.value = false
  }
}

async function quickAdd() {
  const item = newItem.value.trim()
  if (!item) {
    ElMessage.warning('请填写事项')
    return
  }
  adding.value = true
  try {
    await api.createExpense({ item, amount: newAmount.value, category: newCategory.value })
    newItem.value = ''
    newAmount.value = 0
    await load()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    adding.value = false
  }
}

// 子按钮动作：＋记一笔（由悬浮按钮经 provide 下发；「统计」走大面板）
const subAction = inject('subAction')
watch(subAction, (act) => {
  if (!act || act.key !== 'money') return
  if (act.action === 'add') {
    showAdd.value = !showAdd.value
    if (showAdd.value) nextTick(() => addInputRef.value?.focus())
  }
})

onMounted(load)
</script>

<style scoped>
.mel {
  min-height: 80px;
}
.mel-summary {
  display: flex;
  gap: 14px;
  align-items: center;
  font-size: 12px;
  color: #909399;
  margin-bottom: 8px;
}
.mel-summary b {
  color: #67c23a;
  font-weight: 600;
}
.mel-scope {
  margin-left: auto;
  display: flex;
  gap: 8px;
}
.mel-scope span {
  cursor: pointer;
  color: #c0c4cc;
}
.mel-scope span.on {
  color: #67c23a;
  font-weight: 600;
}
.mel-add {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 8px;
}
.mel-addrow {
  display: flex;
  gap: 6px;
}
.mel-empty {
  border: 1px dashed #dcdfe6;
  border-radius: 8px;
  color: #909399;
  font-size: 13px;
  text-align: center;
  padding: 24px 0;
}
.mel-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 10px;
  margin-bottom: 6px;
  background: #fff;
  border: 1px solid #ebeef5;
  border-left: 3px solid #67c23a;
  border-radius: 6px;
}
.mel-cat {
  flex-shrink: 0;
  font-size: 10px;
  color: #67c23a;
  background: #f0f9eb;
  padding: 3px 6px;
  border-radius: 4px;
  line-height: 1;
}
.mel-cat.clickable {
  cursor: pointer;
}
.mel-cat.clickable:hover {
  background: #e1f3d8;
}
.mel-item {
  flex: 1;
  font-size: 13px;
  color: #303133;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.mel-time {
  flex-shrink: 0;
  font-size: 11px;
  color: #909399;
}
.mel-amount {
  flex-shrink: 0;
  font-size: 13px;
  color: #303133;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
</style>
