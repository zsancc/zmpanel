<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import {
  Bookmark,
  ClipboardCopy,
  ClipboardPaste,
  Play,
  Plus,
  Trash2,
  X,
} from 'lucide-vue-next'
import { apiRequest, getStoredToken, wsUrl } from '@/api/client'
import { useThemeStore } from '@/stores/theme'

interface CommandSnippet {
  id: string
  name: string
  category: string
  command: string
  description: string
}

interface TermTab {
  id: string
  title: string
  cwd: string
  connected: boolean
  term: Terminal | null
  fit: FitAddon | null
  ws: WebSocket | null
}

const route = useRoute()
const theme = useThemeStore()

const tabs = ref<TermTab[]>([])
const activeTabId = ref('')
const containerRefs = new Map<string, HTMLElement>()
const copyOnSelect = ref(false)
const snippetsOpen = ref(true)
const snippets = ref<CommandSnippet[]>([])
const toastMsg = ref('')

// Add snippet modal
const addModal = ref(false)
const newSnippet = ref({ name: '', category: 'Docker', command: '', description: '' })

function showToast(msg: string) {
  toastMsg.value = msg
  setTimeout(() => {
    if (toastMsg.value === msg) toastMsg.value = ''
  }, 2200)
}

function getTermColors() {
  const style = getComputedStyle(document.documentElement)
  const bg = style.getPropertyValue('--zm-term-bg').trim() || '#0f172a'
  const fg = style.getPropertyValue('--zm-term-fg').trim() || '#e2e8f0'
  const accent = style.getPropertyValue('--zm-accent').trim() || '#10b981'
  return {
    background: bg,
    foreground: fg,
    cursor: accent,
    selectionBackground: 'rgba(59, 130, 246, 0.35)',
  }
}

function setContainerRef(id: string, el: any) {
  if (el instanceof HTMLElement) {
    containerRefs.set(id, el)
  } else {
    containerRefs.delete(id)
  }
}

async function createTab(cwd = '') {
  const id = `tab-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`
  const tab: TermTab = {
    id,
    title: cwd ? cwd.split('/').filter(Boolean).pop() || '终端' : `终端 ${tabs.value.length + 1}`,
    cwd,
    connected: false,
    term: null,
    fit: null,
    ws: null,
  }
  tabs.value.push(tab)
  activeTabId.value = id

  await nextTick()
  mountTab(tab)
}

function mountTab(tab: TermTab) {
  const el = containerRefs.get(tab.id)
  if (!el) return

  const term = new Terminal({
    cursorBlink: true,
    fontSize: 14,
    fontFamily: '"JetBrains Mono", "Fira Code", Consolas, monospace',
    scrollback: 5000,
    theme: getTermColors(),
  })
  const fit = new FitAddon()
  term.loadAddon(fit)
  term.loadAddon(new WebLinksAddon())
  term.open(el)
  fit.fit()

  // Fix Baota's infamous terminal copy/paste bug:
  // Support Ctrl+Shift+C / Ctrl+Shift+V & Ctrl+C (when text selected) & Ctrl+V
  term.attachCustomKeyEventHandler((ev: KeyboardEvent) => {
    if (ev.type !== 'keydown') return true
    const isCopy = (ev.ctrlKey && ev.shiftKey && ev.code === 'KeyC') || (ev.ctrlKey && !ev.shiftKey && ev.code === 'KeyC' && term.hasSelection())
    if (isCopy) {
      const sel = term.getSelection()
      if (sel) {
        navigator.clipboard.writeText(sel).then(() => showToast('已复制选中内容'))
      }
      return false
    }
    const isPaste = (ev.ctrlKey && ev.shiftKey && ev.code === 'KeyV') || (ev.ctrlKey && !ev.shiftKey && ev.code === 'KeyV')
    if (isPaste) {
      pasteToActive()
      return false
    }
    return true
  })

  term.onSelectionChange(() => {
    if (copyOnSelect.value) {
      const sel = term.getSelection()
      if (sel) {
        navigator.clipboard.writeText(sel).catch(() => {})
      }
    }
  })

  const token = getStoredToken()
  const params = new URLSearchParams({
    cols: String(term.cols),
    rows: String(term.rows),
  })
  if (tab.cwd) params.set('cwd', tab.cwd)
  if (token) params.set('token', token)

  const ws = new WebSocket(`${wsUrl('/terminal/ws')}?${params.toString()}`)
  tab.term = term
  tab.fit = fit
  tab.ws = ws

  ws.onopen = () => {
    tab.connected = true
    term.focus()
  }

  ws.onmessage = (ev) => {
    try {
      const msg = JSON.parse(ev.data)
      if (msg.type === 'output') {
        term.write(msg.data)
      } else if (msg.type === 'error') {
        term.writeln(`\r\n\x1b[31m[错误] ${msg.message}\x1b[0m`)
      } else if (msg.type === 'exit') {
        tab.connected = false
        term.writeln(`\r\n\x1b[33m[${msg.message}]\x1b[0m`)
      }
    } catch {
      term.write(String(ev.data))
    }
  }

  ws.onclose = () => {
    tab.connected = false
  }

  term.onData((data) => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'input', data }))
    }
  })

  term.onResize(({ cols, rows }) => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify({ type: 'resize', cols, rows }))
    }
  })
}

function getActiveTab(): TermTab | undefined {
  return tabs.value.find((t) => t.id === activeTabId.value)
}

function sendInputToActive(text: string) {
  const tab = getActiveTab()
  if (!tab || !tab.ws || tab.ws.readyState !== WebSocket.OPEN) {
    showToast('当前终端未连接')
    return
  }
  tab.ws.send(JSON.stringify({ type: 'input', data: text }))
  tab.term?.focus()
}

function copySelectionFromActive() {
  const tab = getActiveTab()
  const sel = tab?.term?.getSelection()
  if (!sel) {
    showToast('请先在终端中拖拽选中文本')
    return
  }
  navigator.clipboard.writeText(sel).then(() => showToast('已复制到剪贴板'))
}

async function pasteToActive() {
  try {
    const text = await navigator.clipboard.readText()
    if (text) {
      sendInputToActive(text)
    }
  } catch {
    showToast('无法读取剪贴板，请确认浏览器已授予剪贴板权限')
  }
}

function closeTab(id: string) {
  const idx = tabs.value.findIndex((t) => t.id === id)
  if (idx === -1) return
  const tab = tabs.value[idx]
  tab.ws?.close()
  tab.term?.dispose()
  tabs.value.splice(idx, 1)
  if (activeTabId.value === id) {
    activeTabId.value = tabs.value[Math.max(0, idx - 1)]?.id || ''
  }
}

function fitActive() {
  nextTick(() => {
    const tab = getActiveTab()
    tab?.fit?.fit()
    tab?.term?.focus()
  })
}

async function loadSnippets() {
  try {
    snippets.value = await apiRequest<CommandSnippet[]>('/terminal/snippets')
  } catch {
    snippets.value = []
  }
}

async function saveSnippet() {
  if (!newSnippet.value.name.trim() || !newSnippet.value.command.trim()) return
  await apiRequest('/terminal/snippets', {
    method: 'POST',
    body: JSON.stringify(newSnippet.value),
  })
  newSnippet.value = { name: '', category: 'Docker', command: '', description: '' }
  addModal.value = false
  await loadSnippets()
  showToast('已保存到命令收藏夹')
}

async function removeSnippet(id: string) {
  await apiRequest(`/terminal/snippets/${id}`, { method: 'DELETE' })
  await loadSnippets()
}

watch(activeTabId, fitActive)
watch(snippetsOpen, () => setTimeout(fitActive, 120))
watch(
  () => theme.themeId,
  () => {
    const colors = getTermColors()
    for (const t of tabs.value) {
      if (t.term) t.term.options.theme = colors
    }
  },
)

onMounted(() => {
  loadSnippets()
  const initCwd = typeof route.query.cwd === 'string' ? route.query.cwd : ''
  createTab(initCwd)
  window.addEventListener('resize', fitActive)
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', fitActive)
  for (const t of tabs.value) {
    t.ws?.close()
    t.term?.dispose()
  }
})
</script>

<template>
  <div class="flex flex-col h-[calc(100vh-6.5rem)] gap-3">
    <!-- Top bar -->
    <div class="zm-card px-3 py-2 flex flex-wrap items-center justify-between gap-2">
      <div class="flex items-center gap-1.5 overflow-x-auto">
        <div
          v-for="tab in tabs"
          :key="tab.id"
          class="flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs cursor-pointer border"
          :style="{
            borderColor: activeTabId === tab.id ? 'var(--zm-accent)' : 'var(--zm-border)',
            background: activeTabId === tab.id ? 'var(--zm-accent-soft)' : 'var(--zm-surface-elevated)',
          }"
          @click="activeTabId = tab.id"
        >
          <span
            class="h-2 w-2 rounded-full"
            :class="tab.connected ? 'bg-emerald-500' : 'bg-rose-500'"
          />
          <span class="font-medium">{{ tab.title }}</span>
          <button class="opacity-60 hover:opacity-100" @click.stop="closeTab(tab.id)">
            <X class="h-3 w-3" />
          </button>
        </div>
        <button class="zm-btn !py-1 !px-2 text-xs" @click="createTab()">
          <Plus class="h-3.5 w-3.5" /> 新标签
        </button>
      </div>

      <div class="flex items-center gap-2">
        <span v-if="toastMsg" class="text-xs font-medium" style="color: var(--zm-accent)">
          {{ toastMsg }}
        </span>
        <label class="flex items-center gap-1.5 text-xs cursor-pointer">
          <input v-model="copyOnSelect" type="checkbox" />
          选中即复制
        </label>
        <button class="zm-btn !py-1 !px-2 text-xs" title="复制选中文本 (Ctrl+Shift+C)" @click="copySelectionFromActive">
          <ClipboardCopy class="h-3.5 w-3.5" /> 复制
        </button>
        <button class="zm-btn !py-1 !px-2 text-xs" title="粘贴剪贴板 (Ctrl+Shift+V)" @click="pasteToActive">
          <ClipboardPaste class="h-3.5 w-3.5" /> 粘贴
        </button>
        <button class="zm-btn !py-1 !px-2 text-xs" @click="snippetsOpen = !snippetsOpen">
          <Bookmark class="h-3.5 w-3.5" /> 命令收藏夹
        </button>
      </div>
    </div>

    <!-- Main Split: Terminal + Command Snippets -->
    <div class="flex-1 min-h-0 flex gap-3">
      <div
        class="flex-1 min-w-0 rounded-xl overflow-hidden border p-2 relative"
        style="background: var(--zm-term-bg); border-color: var(--zm-border)"
      >
        <div
          v-for="tab in tabs"
          :key="tab.id"
          v-show="activeTabId === tab.id"
          :ref="(el) => setContainerRef(tab.id, el)"
          class="h-full w-full"
        />
        <div
          v-if="tabs.length === 0"
          class="h-full flex flex-col items-center justify-center gap-3 text-xs opacity-70"
        >
          <div>没有打开的终端会话</div>
          <button class="zm-btn zm-btn-primary" @click="createTab()">打开新终端</button>
        </div>
      </div>

      <!-- Command Snippets Sidebar -->
      <div
        v-if="snippetsOpen"
        class="w-72 shrink-0 zm-card p-3 flex flex-col gap-2.5 overflow-hidden"
      >
        <div class="flex items-center justify-between">
          <span class="text-xs font-semibold">常用命令收藏夹</span>
          <button class="zm-btn zm-btn-primary !py-1 !px-2 text-[11px]" @click="addModal = true">
            <Plus class="h-3 w-3" /> 添加
          </button>
        </div>
        <div class="flex-1 overflow-y-auto space-y-2 pr-0.5">
          <div
            v-for="s in snippets"
            :key="s.id"
            class="rounded-lg p-2.5 space-y-1.5"
            style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
          >
            <div class="flex items-center justify-between gap-1">
              <span class="text-xs font-semibold truncate">{{ s.name }}</span>
              <span class="text-[10px] px-1.5 py-0.5 rounded opacity-80" style="background: var(--zm-surface)">
                {{ s.category }}
              </span>
            </div>
            <pre
              class="text-[11px] font-mono p-1.5 rounded overflow-x-auto m-0"
              style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
            >{{ s.command }}</pre>
            <div class="flex items-center justify-between pt-1">
              <button
                class="zm-btn !py-0.5 !px-2 text-[11px]"
                @click="sendInputToActive(s.command)"
              >
                填入命令
              </button>
              <div class="flex gap-1">
                <button
                  class="zm-btn zm-btn-primary !py-0.5 !px-2 text-[11px]"
                  @click="sendInputToActive(s.command + '\n')"
                >
                  <Play class="h-3 w-3" /> 执行
                </button>
                <button class="zm-btn zm-btn-danger !py-0.5 !px-1.5" @click="removeSnippet(s.id)">
                  <Trash2 class="h-3 w-3" />
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Add Snippet Modal -->
    <div v-if="addModal" class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div class="zm-card w-full max-w-md p-5 space-y-3">
        <div class="font-semibold text-sm">新增常用命令</div>
        <div>
          <label class="text-xs block mb-1" style="color: var(--zm-text-muted)">命令名称</label>
          <input v-model="newSnippet.name" class="zm-input" placeholder="例如：Compose 重新构建启动" />
        </div>
        <div>
          <label class="text-xs block mb-1" style="color: var(--zm-text-muted)">分类</label>
          <input v-model="newSnippet.category" class="zm-input" placeholder="Docker / 系统运维 / Nginx" />
        </div>
        <div>
          <label class="text-xs block mb-1" style="color: var(--zm-text-muted)">命令内容</label>
          <textarea v-model="newSnippet.command" rows="3" class="zm-input font-mono text-xs" placeholder="docker compose up -d --build" />
        </div>
        <div class="flex justify-end gap-2 pt-2">
          <button class="zm-btn" @click="addModal = false">取消</button>
          <button class="zm-btn zm-btn-primary" @click="saveSnippet">保存</button>
        </div>
      </div>
    </div>
  </div>
</template>
