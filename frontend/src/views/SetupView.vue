<template>
  <div class="setup-page">
    <el-card class="setup-card">
      <h2>欢迎使用 Latte</h2>
      <p class="sub">首次使用需要连接你的 Notion，请按以下步骤操作：</p>

      <ol class="steps">
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
        <li>在下方粘贴 Token 和页面 URL，点击「完成配置」。</li>
      </ol>

      <el-form :model="form" label-width="90px" @submit.prevent>
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
    </el-card>
  </div>
</template>

<script setup>
import { reactive, ref } from 'vue'
import { ElMessage } from 'element-plus'
import { api } from '../api'

const emit = defineEmits(['done'])

const form = reactive({ token: '', page_url: '' })
const error = ref('')
const submitting = ref(false)
const testing = ref(false)
const testResult = ref(null)

async function testConfig() {
  if (!form.token.trim() || !form.page_url.trim()) {
    error.value = '请填写 Token 和页面 URL'
    return
  }
  error.value = ''
  testResult.value = null
  testing.value = true
  try {
    const res = await api.verifySetup(form.token.trim(), form.page_url.trim())
    if (!res.token_valid) {
      testResult.value = {
        type: 'error',
        title: 'Token 无效或无权访问',
        detail: '请检查 Token 是否正确，以及 Integration 是否已连接到目标页面。',
        databases: [],
        missing: ['时间碎片', '金钱记录', '项目管理'],
      }
    } else if (res.error) {
      // token 有效但对象不可访问/类型不对（如数据库位于工作区根目录）
      testResult.value = {
        type: 'error',
        title: '无法访问该页面',
        detail: res.error,
        databases: [],
        missing: ['时间碎片', '金钱记录', '项目管理'],
      }
    } else {
      // token 有效且已定位到根页面
      const climbed = res.is_database
        ? '检测到你填的 URL 指向的是一个数据库，已自动向上定位到它的父页面。'
        : ''
      if (res.missing && res.missing.length) {
        testResult.value = {
          type: 'warning',
          title: 'Token 有效，但部分数据库未找到',
          detail: (climbed ? climbed + ' ' : '') +
            '以下数据库未在根页面下找到，点击「完成配置」将自动创建缺失的数据库。',
          databases: res.databases,
          missing: res.missing,
        }
      } else {
        testResult.value = {
          type: 'success',
          title: '配置正确！所有数据库已就绪',
          detail: (climbed ? climbed + ' ' : '') +
            'Token 有效，已找到全部 3 个数据库与知识库页面。',
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
async function submit() {
  if (!form.token.trim() || !form.page_url.trim()) {
    error.value = '请填写 Token 和页面 URL'
    return
  }
  error.value = ''
  submitting.value = true
  try {
    await api.setup(form.token.trim(), form.page_url.trim())
    ElMessage.success('配置成功')
    emit('done')
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
  align-items: center;
  padding: 24px;
}

.setup-card {
  max-width: 640px;
  width: 100%;
}

.sub {
  color: #909399;
}

.steps {
  line-height: 1.9;
  padding-left: 20px;
}

.err {
  margin-bottom: 18px;
}
</style>
