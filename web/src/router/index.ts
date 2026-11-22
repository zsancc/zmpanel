import { createRouter, createWebHashHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: '/login',
      name: 'login',
      component: () => import('@/views/LoginView.vue'),
      meta: { public: true },
    },
    {
      path: '/',
      component: () => import('@/layouts/MainLayout.vue'),
      children: [
        { path: '', name: 'dashboard', component: () => import('@/views/DashboardView.vue') },
        { path: 'monitor', name: 'monitor', component: () => import('@/views/MonitorView.vue') },
        { path: 'files', name: 'files', component: () => import('@/views/FilesView.vue') },
        { path: 'terminal', name: 'terminal', component: () => import('@/views/TerminalView.vue') },
        { path: 'websites', name: 'websites', component: () => import('@/views/WebsitesView.vue') },
        { path: 'docker', name: 'docker', component: () => import('@/views/DockerView.vue') },
        { path: 'ssl', name: 'ssl', component: () => import('@/views/SslView.vue') },
        { path: 'store', name: 'store', component: () => import('@/views/StoreView.vue') },
        { path: 'backup', name: 'backup', component: () => import('@/views/BackupView.vue') },
        { path: 'firewall', name: 'firewall', component: () => import('@/views/FirewallView.vue') },
        { path: 'security', name: 'security', component: () => import('@/views/SecurityView.vue') },
        { path: 'logs', name: 'logs', component: () => import('@/views/LogsView.vue') },
        { path: 'cron', name: 'cron', component: () => import('@/views/CronView.vue') },
        { path: 'nodes', name: 'nodes', component: () => import('@/views/NodesView.vue') },
        { path: 'settings', name: 'settings', component: () => import('@/views/SettingsView.vue') },
        { path: 'module/:slug', name: 'roadmap', component: () => import('@/views/RoadmapView.vue') },
      ],
    },
  ],
})

router.beforeEach(async (to) => {
  const auth = useAuthStore()
  if (!auth.initialized) {
    await auth.fetchMe()
  }
  if (!to.meta.public && !auth.profile) {
    return { name: 'login' }
  }
  if (to.name === 'login' && auth.profile) {
    return { name: 'dashboard' }
  }
  return true
})

export default router
