import DefaultTheme from 'vitepress/theme'
import Layout from './Layout.vue'
import DemoVideo from './DemoVideo.vue'
import './style.css'

export default {
  extends: DefaultTheme,
  Layout,
  enhanceApp({ app }) {
    app.component('DemoVideo', DemoVideo)
  },
}
