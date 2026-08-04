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
  sync: () => unwrap(http.post('/sync')),

  getEvents: (date) => unwrap(http.get('/events', { params: { date } })),
  createEvent: (data) => unwrap(http.post('/events', data)),
  startEvent: (data) => unwrap(http.post('/events/start', data || {})),
  stopEvent: (id, content, tag) => unwrap(http.post(`/events/${id}/stop`, { content, tag })),
  updateEvent: (id, data) => unwrap(http.put(`/events/${id}`, data)),
  deleteEvent: (id) => unwrap(http.delete(`/events/${id}`)),

  getTimeReport: (period, date) => unwrap(http.get('/reports/time', { params: { period, date } })),

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
  rolloverTasks: (from, to) => unwrap(http.post('/tasks/rollover', { from, to })),
  pomodoroTask: (id) => unwrap(http.post(`/tasks/${id}/pomodoro`)),
  getOngoingEvent: () => unwrap(http.get('/events/ongoing')),
}
