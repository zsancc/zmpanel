<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import {
  Activity,
  Box,
  Clock,
  FileCode2,
  FolderOpen,
  Globe,
  HardDriveUpload,
  LayoutDashboard,
  Lock,
  LogOut,
  Menu,
  Palette,
  Power,
  RotateCw,
  ScrollText,
  Server,
  Settings,
  ShieldAlert,
  ShieldCheck,
  Store,
  Terminal,
  X,
} from 'lucide-vue-next'
import { apiRequest } from '@/api/client'
import { useAuthStore } from '@/stores/auth'
import { ACCENT_PRESETS, THEME_PRESETS, useThemeStore } from '@/stores/theme'

interface NodeSummary {
  id: string
  name: string
  mode: string
  status: string
  os_name: string
  arch: string
  cpu_usage: number
  mem_used_pct: number
  is_local: boolean
}

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()
const theme = useThemeStore()

const collapsed = ref(false)
const mobileOpen = ref(false)
const themePickerOpen = ref(false)
const powerMenuOpen = ref(false)
const powerBusyText = ref('')
const nodes = ref<NodeSummary[]>([])
const activeNodeId = ref('local')

interface NavItem {
  label: string
  to: string
  icon: any
  ready: boolean
  stage?: string
}

const navGroups: { label: string; items: NavItem[] }[] = [
  {
    label: '核心运维',
    items: [
      { label: '概览', to: '/', icon: LayoutDashboard, ready: true },
      { label: '监控与进程', to: '/monitor', icon: Activity, ready: true },
      { label: '文件管理', to: '/files', icon: FolderOpen, ready: true },
      { label: 'Web 终端', to: '/terminal', icon: Terminal, ready: true },
    ],
  },
  {
    label: '应用与容器',
    items: [
      { label: '网站管理', to: '/websites', icon: Globe, ready: true },
      { label: 'Docker 工作区', to: '/docker', icon: Box, ready: true },
      { label: 'SSL 证书', to: '/ssl', icon: Lock, ready: true },
      { label: '软件商店', to: '/store', icon: Store, ready: true },
      { label: '备份与迁移', to: '/backup', icon: HardDriveUpload, ready: true },
    ],
  },
  {
    label: '安全与集群',
    items: [
      { label: '防火墙', to: '/firewall', icon: ShieldCheck, ready: true },
      { label: '安全中心', to: '/security', icon: ShieldAlert, ready: true },
      { label: '日志中心', to: '/logs', icon: ScrollText, ready: true },
      { label: '计划任务', to: '/cron', icon: Clock, ready: true },
      { label: '节点集群', to: '/nodes', icon: Server, ready: true },
    ],
  },
  {
    label: '面板',
    items: [{ label: '系统设置', to: '/settings', icon: Settings, ready: true }],
  },
]

const currentNode = computed(() => nodes.value.find((n) => n.id === activeNodeId.value))

async function loadNodes() {
  try {
    nodes.value = await apiRequest<NodeSummary[]>('/nodes')
  } catch {
    nodes.value = []
  }
}

async function handleLogout() {
  await auth.logout()
  router.replace({ name: 'login' })
}

async function triggerPower(target: 'panel' | 'server') {
  powerMenuOpen.value = false
  const msg =
    target === 'panel'
      ? '确定要重启 zmpanel 面板服务吗？（约 2~3 秒后自动重连，不影响网站和容器运行）'
      : '危险操作：确定要立即重启整台服务器系统吗？（所有正在运行的服务与 SSH 连接都会中断 30~60 秒）'
  if (!window.confirm(msg)) return

  try {
    const res = await apiRequest<string>('/system/power', {
      method: 'POST',
      body: JSON.stringify({ target }),
    })
    powerBusyText.value = res
    // Poll auth status until server comes back online
    const waitInitial = target === 'panel' ? 2000 : 10000
    setTimeout(() => {
      const poll = window.setInterval(async () => {
        try {
          await apiRequest('/auth/status')
          window.clearInterval(poll)
          powerBusyText.value = ''
          window.location.reload()
        } catch {
          // still restarting
        }
      }, 2000)
    }, waitInitial)
  } catch (err: any) {
    window.alert(err?.message || '触发重启失败')
  }
}

function isActive(to: string) {
  if (to === '/') return route.path === '/'
  return route.path === to || route.path.startsWith(`${to}/`)
}

onMounted(() => {
  loadNodes()
})
</script>

<template>
  <div class="flex h-screen overflow-hidden" style="background: var(--zm-bg); color: var(--zm-text)">
    <!-- Mobile backdrop -->
    <div
      v-if="mobileOpen"
      class="fixed inset-0 z-30 bg-black/50 lg:hidden"
      @click="mobileOpen = false"
    />

    <!-- Sidebar -->
    <aside
      :class="[
        'fixed inset-y-0 left-0 z-40 flex flex-col transition-all duration-200 lg:static',
        collapsed ? 'w-16' : 'w-60',
        mobileOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0',
      ]"
      style="background: var(--zm-sidebar); color: var(--zm-sidebar-text); border-right: 1px solid rgba(255,255,255,0.06)"
    >
      <!-- Brand -->
      <div class="flex h-14 items-center justify-between px-4 border-b border-white/10">
        <div class="flex items-center gap-2.5 overflow-hidden">
          <div
            class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg font-bold text-white shadow"
            style="background: var(--zm-accent)"
          >
            <FileCode2 class="h-4 w-4" />
          </div>
          <div v-if="!collapsed" class="truncate">
            <div class="text-sm font-semibold tracking-wide text-white">zmpanel</div>
            <div class="text-[11px] opacity-60">v{{ auth.profile?.version || '0.1.0' }} · 无外联</div>
          </div>
        </div>
        <button class="lg:hidden p-1 opacity-70 hover:opacity-100" @click="mobileOpen = false">
          <X class="h-4 w-4" />
        </button>
      </div>

      <!-- Nav -->
      <nav class="flex-1 overflow-y-auto px-2.5 py-3 space-y-4">
        <div v-for="group in navGroups" :key="group.label">
          <div
            v-if="!collapsed"
            class="px-2.5 mb-1.5 text-[11px] font-medium uppercase tracking-wider"
            style="color: var(--zm-sidebar-muted)"
          >
            {{ group.label }}
          </div>
          <div class="space-y-0.5">
            <router-link
              v-for="item in group.items"
              :key="item.to"
              :to="item.to"
              class="flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm transition-colors"
              :style="
                isActive(item.to)
                  ? { background: 'var(--zm-accent)', color: '#ffffff' }
                  : { color: 'var(--zm-sidebar-text)' }
              "
              @click="mobileOpen = false"
            >
              <component :is="item.icon" class="h-4 w-4 shrink-0" />
              <span v-if="!collapsed" class="flex-1 truncate">{{ item.label }}</span>
              <span
                v-if="!collapsed && !item.ready"
                class="rounded px-1.5 py-0.5 text-[10px] font-medium bg-white/10 opacity-75"
              >
                {{ item.stage }}
              </span>
            </router-link>
          </div>
        </div>
      </nav>

      <!-- Footer -->
      <div class="border-t border-white/10 p-2.5">
        <button
          class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-sm opacity-80 hover:opacity-100 hover:bg-white/10 transition"
          @click="handleLogout"
        >
          <LogOut class="h-4 w-4 shrink-0" />
          <span v-if="!collapsed">退出登录</span>
        </button>
      </div>
    </aside>

    <!-- Main Area -->
    <div class="flex flex-1 flex-col min-w-0 overflow-hidden">
      <!-- Topbar -->
      <header
        class="flex h-14 shrink-0 items-center justify-between px-4 gap-3"
        style="background: var(--zm-surface); border-bottom: 1px solid var(--zm-border)"
      >
        <div class="flex items-center gap-3 min-w-0">
          <button
            class="zm-btn !p-1.5"
            title="展开/收起侧边栏"
            @click="mobileOpen = !mobileOpen; collapsed = !collapsed"
          >
            <Menu class="h-4 w-4" />
          </button>

          <!-- Node selector -->
          <div class="flex items-center gap-2">
            <span class="h-2 w-2 rounded-full bg-emerald-500 shrink-0" />
            <select
              v-model="activeNodeId"
              class="zm-input !w-auto !py-1 !text-xs font-medium"
            >
              <option v-for="n in nodes" :key="n.id" :value="n.id">
                {{ n.name }} ({{ n.arch }})
              </option>
              <option v-if="nodes.length === 0" value="local">本机节点</option>
            </select>
            <span
              v-if="currentNode"
              class="hidden md:inline-block text-xs truncate max-w-xs"
              style="color: var(--zm-text-muted)"
            >
              {{ currentNode.os_name }}
            </span>
          </div>
        </div>

        <div class="flex items-center gap-2 relative">
          <button
            class="zm-btn !py-1.5 !px-2.5 text-xs"
            title="重启面板或服务器"
            @click="powerMenuOpen = !powerMenuOpen; themePickerOpen = false"
          >
            <Power class="h-4 w-4 text-amber-400" />
            <span class="hidden sm:inline">重启</span>
          </button>

          <button
            class="zm-btn !py-1.5 !px-2.5 text-xs"
            @click="themePickerOpen = !themePickerOpen; powerMenuOpen = false"
          >
            <Palette class="h-4 w-4" style="color: var(--zm-accent)" />
            <span class="hidden sm:inline">主题</span>
          </button>

          <!-- Quick power dropdown -->
          <div
            v-if="powerMenuOpen"
            class="absolute right-16 top-11 z-50 w-52 p-2 shadow-xl zm-card space-y-1 text-xs"
          >
            <button
              class="w-full flex items-center gap-2 rounded-lg px-2.5 py-2 text-left hover:bg-white/10 transition cursor-pointer"
              @click="triggerPower('panel')"
            >
              <RotateCw class="h-3.5 w-3.5" style="color: var(--zm-accent)" />
              <div>
                <div class="font-medium">重启面板</div>
                <div class="text-[10px] opacity-65">仅重启 zmpanel，2~3 秒恢复</div>
              </div>
            </button>
            <button
              class="w-full flex items-center gap-2 rounded-lg px-2.5 py-2 text-left hover:bg-rose-500/15 transition cursor-pointer"
              style="color: var(--zm-danger)"
              @click="triggerPower('server')"
            >
              <Power class="h-3.5 w-3.5" />
              <div>
                <div class="font-medium">重启服务器系统</div>
                <div class="text-[10px] opacity-75">整机 reboot（内核补丁生效）</div>
              </div>
            </button>
          </div>

          <div
            class="hidden sm:flex items-center gap-2 rounded-lg px-2.5 py-1 text-xs"
            style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
          >
            <span class="font-medium">{{ auth.profile?.username || 'admin' }}</span>
            <span
              v-if="auth.profile?.totp_enabled"
              class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
              style="background: var(--zm-accent-soft); color: var(--zm-accent)"
            >
              2FA
            </span>
          </div>

          <!-- Quick theme dropdown -->
          <div
            v-if="themePickerOpen"
            class="absolute right-0 top-11 z-50 w-72 p-3 shadow-xl zm-card"
          >
            <div class="flex items-center justify-between mb-2">
              <span class="text-xs font-semibold">界面主题（6 套）</span>
              <button class="text-xs opacity-60 hover:opacity-100" @click="themePickerOpen = false">
                关闭
              </button>
            </div>
            <div class="grid grid-cols-2 gap-2 mb-3">
              <button
                v-for="t in THEME_PRESETS"
                :key="t.id"
                class="flex items-center gap-2 rounded-lg p-2 text-left text-xs border transition cursor-pointer"
                :style="{
                  borderColor: theme.themeId === t.id ? 'var(--zm-accent)' : 'var(--zm-border)',
                  background: theme.themeId === t.id ? 'var(--zm-accent-soft)' : 'var(--zm-surface-elevated)',
                }"
                @click="theme.setTheme(t.id)"
              >
                <span
                  class="flex h-5 w-5 shrink-0 overflow-hidden rounded border border-white/20"
                  :style="{ background: t.previewBg }"
                >
                  <span class="w-2 h-full" :style="{ background: t.previewSidebar }" />
                </span>
                <span class="truncate font-medium">{{ t.name }}</span>
              </button>
            </div>
            <div class="text-xs font-semibold mb-1.5">强调色</div>
            <div class="flex items-center gap-2">
              <button
                v-for="a in ACCENT_PRESETS"
                :key="a.id"
                :title="a.name"
                class="h-6 w-6 rounded-full cursor-pointer transition-transform"
                :style="{
                  background: a.color,
                  transform: theme.accentId === a.id ? 'scale(1.18)' : 'scale(1)',
                  outline: theme.accentId === a.id ? '2px solid var(--zm-text)' : 'none',
                  outlineOffset: '2px',
                }"
                @click="theme.setAccent(a.id)"
              />
            </div>
          </div>
        </div>
      </header>

      <!-- Page Content -->
      <main class="flex-1 overflow-y-auto p-4 lg:p-6">
        <div
          v-if="powerBusyText"
          class="mb-4 zm-card p-3 text-xs flex items-center gap-2"
          style="border-color: var(--zm-warning); color: var(--zm-warning)"
        >
          <RotateCw class="h-3.5 w-3.5 animate-spin shrink-0" />
          <span>{{ powerBusyText }}（页面将在恢复连通后自动刷新）</span>
        </div>
        <router-view />
      </main>
    </div>
  </div>
</template>
