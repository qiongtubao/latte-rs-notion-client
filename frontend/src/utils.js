import dayjs from 'dayjs'

export function fmtTs(ts) {
  return ts ? dayjs.unix(ts).format('YYYY-MM-DD HH:mm') : '—'
}

export function fmtTime(ts) {
  return ts ? dayjs.unix(ts).format('HH:mm') : '—'
}

export function fmtDate(ts) {
  return ts ? dayjs.unix(ts).format('YYYY-MM-DD') : '—'
}

// 秒 → "HH:MM:SS"（实时计时用）
export function fmtHMS(seconds) {
  const s = Math.max(0, Math.floor(seconds))
  const h = String(Math.floor(s / 3600)).padStart(2, '0')
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0')
  const ss = String(s % 60).padStart(2, '0')
  return `${h}:${m}:${ss}`
}

// 秒 → 友好的时长（如 1h30m / 45m / 2.5h）
export function fmtDuration(seconds) {
  const s = Math.max(0, Math.floor(seconds || 0))
  if (s === 0) return '0m'
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (h > 0 && m > 0) return `${h}h${m}m`
  if (h > 0) return `${h}h`
  if (m > 0) return `${m}m`
  return `${s}s`
}

// 日历格子用：小时保留一位小数
export function fmtHours(seconds) {
  const h = (seconds || 0) / 3600
  return `${Math.round(h * 10) / 10}h`
}

export function fmtMoney(yuan) {
  return `¥${Number(yuan || 0).toFixed(2)}`
}

export function fmtCents(cents) {
  return `¥${(Number(cents || 0) / 100).toFixed(2)}`
}

export const EVENT_TAGS = ['工作', '运动', '生活', '学习', '看书']

export const TAG_TYPES = {
  工作: 'primary',
  运动: 'success',
  生活: 'warning',
  学习: 'danger',
  看书: 'info',
}

// 时间线色块等需要实色的场景
export const TAG_COLORS = {
  工作: '#409eff',
  运动: '#67c23a',
  生活: '#e6a23c',
  学习: '#f56c6c',
  看书: '#909399',
}

export function tagType(tag) {
  return TAG_TYPES[tag] || 'info'
}

export const EXPENSE_CATEGORIES = ['餐饮', '交通', '购物', '娱乐', '其他']

export const PROJECT_STATUSES = ['暂存', '待办', '进行中', '已完成', '暂停']

// el-tag type 映射
export const PROJECT_STATUS_TYPES = {
  暂存: 'info',
  待办: 'primary',
  进行中: 'success',
  已完成: 'success',
  暂停: 'warning',
}

// 看板列圆点配色
export const PROJECT_STATUS_COLORS = {
  暂存: '#909399',
  待办: '#409eff',
  进行中: '#67c23a',
  已完成: '#13c2c2',
  暂停: '#e6a23c',
}

export const IDEA_TAGS = ['灵感', '待办', '读书', '问题', '其他']

export const IDEA_TAG_TYPES = {
  灵感: 'warning',
  待办: 'primary',
  读书: 'success',
  问题: 'danger',
  其他: 'info',
}

// 相对时间：x 分钟前 / x 小时前 / x 天前，超过 7 天回退到完整时间
export function relTime(ts) {
  if (!ts) return '—'
  const diff = dayjs().unix() - ts
  if (diff < 60) return '刚刚'
  if (diff < 3600) return `${Math.floor(diff / 60)} 分钟前`
  if (diff < 86400) return `${Math.floor(diff / 3600)} 小时前`
  if (diff < 7 * 86400) return `${Math.floor(diff / 86400)} 天前`
  return fmtTs(ts)
}
