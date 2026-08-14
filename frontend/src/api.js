import axios from 'axios'

const http = axios.create({ baseURL: '/api', timeout: 15000 })

function unwrap(promise) {
  return promise.then((res) => res.data).catch((err) => {
    const msg = err.response?.data?.error || err.message || '请求失败'
    const e = new Error(msg)
    e.status = err.response?.status
    throw e
  })
}

export const api = {
  getStatus: () => unwrap(http.get('/status')),
  setup: (token, page_url) => unwrap(http.post('/setup', { token, page_url })),
  verifySetup: (token, page_url) => unwrap(http.post('/setup/verify', { token, page_url })),
  pullFromNotion: () => unwrap(http.post('/sync/pull')),
  sync: () => unwrap(http.post('/sync')),
  resetLocalData: () => unwrap(http.post('/data/reset')),
  toggleReminders: (enabled) => unwrap(http.post('/reminders/toggle', { enabled })),
  search: (q) => unwrap(http.get('/search', { params: { q } })),
  importData: (payload) => unwrap(http.post('/import', payload)),

  getEvents: (date) => unwrap(http.get('/events', { params: { date } })),
  createEvent: (data) => unwrap(http.post('/events', data)),
  startEvent: (data) => unwrap(http.post('/events/start', data || {})),
  stopEvent: (id, content, tag) => unwrap(http.post(`/events/${id}/stop`, { content, tag })),
  updateEvent: (id, data) => unwrap(http.put(`/events/${id}`, data)),
  deleteEvent: (id) => unwrap(http.delete(`/events/${id}`)),

  getTimeReport: (period, date) => unwrap(http.get('/reports/time', { params: { period, date } })),
  getReportStats: (period, date) => unwrap(http.get('/reports/stats', { params: { period, date } })),

  getExpenses: (from, to) => unwrap(http.get('/expenses', { params: { from, to } })),
  createExpense: (data) => unwrap(http.post('/expenses', data)),
  updateExpense: (id, data) => unwrap(http.put(`/expenses/${id}`, data)),
  deleteExpense: (id) => unwrap(http.delete(`/expenses/${id}`)),
  getExpenseSummary: () => unwrap(http.get('/expenses/summary')),

  getCalendar: (month) => unwrap(http.get('/calendar', { params: { month } })),
  getCalendarDay: (date) => unwrap(http.get('/calendar/day', { params: { date } })),

  recognizeEvents: (image_base64, media_type, date) =>
    unwrap(http.post('/ai/recognize-events', { image_base64, media_type, date })),

  getProjects: () => unwrap(http.get('/projects')),
  createProject: (data) => unwrap(http.post('/projects', data)),
  updateProject: (id, data) => unwrap(http.put(`/projects/${id}`, data)),
  deleteProject: (id) => unwrap(http.delete(`/projects/${id}`)),

  getNotesTree: () => unwrap(http.get('/notes/tree')),
  searchNotes: (q) => unwrap(http.get('/notes/search', { params: { q } })),
  getNote: (id) => unwrap(http.get(`/notes/${id}`)),
  createNote: (data) => unwrap(http.post('/notes', data)),
  updateNote: (id, data) => unwrap(http.put(`/notes/${id}`, data)),
  deleteNote: (id) => unwrap(http.delete(`/notes/${id}`)),

  getIdeas: (tag) => unwrap(http.get('/ideas', { params: tag ? { tag } : {} })),
  createIdea: (data) => unwrap(http.post('/ideas', data)),
  updateIdea: (id, data) => unwrap(http.put(`/ideas/${id}`, data)),
  deleteIdea: (id) => unwrap(http.delete(`/ideas/${id}`)),

  getTasks: (date) => unwrap(http.get('/tasks', { params: date ? { date } : {} })),
  getUnfinishedTasks: () => unwrap(http.get('/tasks', { params: { unfinished: 1 } })),
  createTask: (data) => unwrap(http.post('/tasks', data)),
  updateTask: (id, data) => unwrap(http.put(`/tasks/${id}`, data)),
  deleteTask: (id) => unwrap(http.delete(`/tasks/${id}`)),
  getPomodoro: () => unwrap(http.get('/pomodoro')),
  getPomodoroStats: (days) => unwrap(http.get('/pomodoro/stats', { params: { days } })),
  rolloverTasks: (from, to) => unwrap(http.post('/tasks/rollover', { from, to })),
  pomodoroTask: (id, minutes) => unwrap(http.post(`/tasks/${id}/pomodoro`, minutes ? { minutes } : {})),
  cancelPomodoro: () => unwrap(http.post('/pomodoro/cancel')),
  quickEntry: (text) => unwrap(http.post('/ai/quick-entry', { text })),
  aiAssist: (panel, text, opts = {}) => unwrap(http.post('/ai/assist', {
    panel, text,
    context: opts.context || '',
    mode: opts.mode || '',
    length: opts.length || '',
    date: opts.date || '',
    focus: opts.focus || '',
  })),
  // 日报/周报/月报：panel=report，text = day|week|month
  aiReport: (period, opts = {}) => unwrap(http.post('/ai/assist', {
    panel: 'report',
    text: period,
    date: opts.date || '',
    focus: opts.focus || '',
    compare: !!opts.compare,
  })),

  getDaily: () => unwrap(http.get('/daily')),
  createDailyItem: (data) => unwrap(http.post('/daily/items', data)),
  updateDailyItem: (id, data) => unwrap(http.put(`/daily/items/${id}`, data)),
  upsertDailyEntry: (data) => unwrap(http.post('/daily/entry', data)),
  getDailyHistory: (days) => unwrap(http.get('/daily/history', { params: { days } })),
  getOngoingEvent: () => unwrap(http.get('/events/ongoing')),
}
