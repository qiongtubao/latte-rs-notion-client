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
        <el-alert v-if="error" :title="error" type="error" :closable="false" class="err" />
        <el-form-item>
          <el-button type="primary" :loading="submitting" @click="submit">完成配置</el-button>
        </el-form-item>
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
