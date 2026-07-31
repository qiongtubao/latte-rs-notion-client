<template>
  <div>
    <!-- 统计卡片 -->
    <el-row :gutter="16">
      <el-col :span="8">
        <el-card shadow="never" class="stat-card">
          <div class="stat-label">本周消费</div>
          <div class="stat-value">{{ fmtMoney(summary.week) }}</div>
        </el-card>
      </el-col>
      <el-col :span="8">
        <el-card shadow="never" class="stat-card">
          <div class="stat-label">本月消费</div>
          <div class="stat-value">{{ fmtMoney(summary.month) }}</div>
        </el-card>
      </el-col>
      <el-col :span="8">
        <el-card shadow="never" class="stat-card">
          <div class="stat-label">本年消费</div>
          <div class="stat-value">{{ fmtMoney(summary.year) }}</div>
        </el-card>
      </el-col>
    </el-row>

    <!-- 当月记录 -->
    <el-card shadow="never" class="section">
      <template #header>
        <div class="section-header">
          <span>{{ monthLabel }} 消费记录</span>
          <el-button type="primary" @click="openCreate">记一笔</el-button>
        </div>
      </template>
      <el-table :data="expenses" empty-text="本月还没有消费记录" v-loading="loading">
        <el-table-column label="时间" width="150">
          <template #default="{ row }">{{ fmtTs(row.ts) }}</template>
        </el-table-column>
        <el-table-column prop="item" label="事项" show-overflow-tooltip />
        <el-table-column label="分类" width="100">
          <template #default="{ row }">
            <el-tag size="small" type="info">{{ row.category }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column label="金额" width="110" align="right">
          <template #default="{ row }">{{ fmtCents(row.amount_cents) }}</template>
        </el-table-column>
        <el-table-column label="操作" width="110">
          <template #default="{ row }">
            <el-button text type="primary" size="small" @click="openEdit(row)">编辑</el-button>
            <el-button text type="danger" size="small" @click="removeExpense(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新建/编辑对话框 -->
    <el-dialog v-model="dialog" :title="form.id ? '编辑记录' : '记一笔'" width="440px">
      <el-form label-width="70px">
        <el-form-item label="事项">
          <el-input v-model="form.item" placeholder="买了什么 / 花在哪？" />
        </el-form-item>
        <el-form-item label="金额">
          <el-input-number v-model="form.amount" :min="0" :precision="2" :step="1" style="width: 100%" />
        </el-form-item>
        <el-form-item label="分类">
          <el-select v-model="form.category" style="width: 100%">
            <el-option v-for="c in EXPENSE_CATEGORIES" :key="c" :label="c" :value="c" />
          </el-select>
        </el-form-item>
        <el-form-item label="时间">
          <el-date-picker v-model="form.ts" type="datetime" style="width: 100%" value-format="X" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialog = false">取消</el-button>
        <el-button type="primary" :loading="saving" @click="save">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup>
import { computed, onMounted, reactive, ref } from 'vue'
import dayjs from 'dayjs'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from '../api'
import { EXPENSE_CATEGORIES, fmtCents, fmtMoney, fmtTs } from '../utils'

const summary = ref({ week: 0, month: 0, year: 0 })
const expenses = ref([])
const loading = ref(false)
const dialog = ref(false)
const saving = ref(false)
const form = reactive({ id: null, item: '', amount: 0, category: '餐饮', ts: null })

const monthStart = dayjs().startOf('month').format('YYYY-MM-DD')
const monthEnd = dayjs().endOf('month').format('YYYY-MM-DD')
const monthLabel = computed(() => dayjs().format('YYYY 年 M 月'))

async function loadAll() {
  loading.value = true
  try {
    const [s, list] = await Promise.all([
      api.getExpenseSummary(),
      api.getExpenses(monthStart, monthEnd),
    ])
    summary.value = s
    expenses.value = list
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    loading.value = false
  }
}

function openCreate() {
  form.id = null
  form.item = ''
  form.amount = 0
  form.category = '餐饮'
  form.ts = String(dayjs().unix())
  dialog.value = true
}

function openEdit(row) {
  form.id = row.id
  form.item = row.item
  form.amount = row.amount_cents / 100
  form.category = row.category || '其他'
  form.ts = String(row.ts)
  dialog.value = true
}

async function save() {
  if (!form.item.trim()) {
    ElMessage.warning('请填写事项')
    return
  }
  saving.value = true
  try {
    const data = {
      item: form.item.trim(),
      amount: form.amount,
      category: form.category,
      ts: form.ts ? Number(form.ts) : undefined,
    }
    if (form.id) {
      await api.updateExpense(form.id, data)
    } else {
      await api.createExpense(data)
    }
    dialog.value = false
    await loadAll()
  } catch (e) {
    ElMessage.error(e.message)
  } finally {
    saving.value = false
  }
}

async function removeExpense(row) {
  try {
    await ElMessageBox.confirm(`删除「${row.item}」？`, '确认删除', { type: 'warning' })
  } catch {
    return
  }
  try {
    await api.deleteExpense(row.id)
    await loadAll()
  } catch (e) {
    ElMessage.error(e.message)
  }
}

onMounted(loadAll)
</script>

<style scoped>
.stat-card {
  text-align: center;
}

.stat-label {
  color: #909399;
  font-size: 13px;
  margin-bottom: 6px;
}

.stat-value {
  font-size: 24px;
  font-weight: 700;
}

.section {
  margin-top: 16px;
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
</style>
