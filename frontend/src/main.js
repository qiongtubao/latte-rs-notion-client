import { createApp } from 'vue'
import ElementPlus from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import 'element-plus/dist/index.css'
import './style.css'
import App from './App.vue'
import BallApp from './BallApp.vue'
import PopupApp from './PopupApp.vue'

// 同一套前端，按 URL query 渲染三种形态：
//   ?ball=<key>  系统级悬浮球窗口（单个球）
//   ?popup=1     快捷操作弹窗（监听后端事件切换视图）
//   其他          主窗口（标签页界面）
const params = new URLSearchParams(location.search)
const ball = params.get('ball')

if (ball) {
  createApp(BallApp, { ballKey: ball }).mount('#app')
} else if (params.has('popup')) {
  createApp(PopupApp).use(ElementPlus, { locale: zhCn }).mount('#app')
} else {
  createApp(App).use(ElementPlus, { locale: zhCn }).mount('#app')
}
