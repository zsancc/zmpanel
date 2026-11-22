<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { Pause, Play, RefreshCw, ScrollText, Search, Trash2 } from 'lucide-vue-next'
import { apiRequest, formatBytes } from '@/api/client'

interface LogSourceItem {
  id: string
  name: string
  category: string
  path: string
  exists: boolean
  size_bytes: number
}

const sources = ref<LogSourceItem[]>([])
const activePath = ref('')
const customPath = ref('')
const keyword = ref('')
const lineCount = ref(300)
const content = ref('')
const fileSize = ref(0)
const matchedLines = ref(0)
const autoFollow = ref(false)
const errorMsg = ref('')
const notice = ref('')
let timer: number | null = null

async function loadSources() {
  try {
    sources.value = await apiRequest<LogSourceItem[]>('/logs/sources')
    if (!activePath.value) {
      const firstExisting = sources.value.find((s) => s.exists) || sources.value[0]
      if (firstExisting) {
        selectPath(firstExisting.path)
      }
    }
  } catch (err: any) {
    errorMsg.value = err?.message || '加载日志源失败'
  }
}

function selectPath(path: string) {
  activePath.value = path
  customPath.value = path
  fetchTail()
}

async function fetchTail() {
  if (!activePath.value.trim()) return
  errorMsg.value = ''
  try {
    const params = new URLSearchParams({
      path: activePath.value.trim(),
      lines: String(lineCount.value),
    })
    if (keyword.value.trim()) params.set('q', keyword.value.trim())
    const res = await apiRequest<{
      path: string
      size_bytes: number
      matched_lines: number
      content: string
    }>(`/logs/tail?${params.toString()}`)
    content.value = res.content
    fileSize.value = res.size_bytes
    matchedLines.value = res.matched_lines
  } catch (err: any) {
    content.value = ''
    errorMsg.value = err?.message || '读取日志失败'
  }
}

async function truncateCurrent() {
  if (!activePath.value) return
  if (!window.confirm(`确定安全清空日志文件 ${activePath.value} 吗？（保留 inode，不中断写入进程）`)) return
  try {
    await apiRequest('/logs/truncate', {
      method: 'POST',
      body: JSON.stringify({ path: activePath.value }),
    })
    notice.value = '日志已清空'
    await fetchTail()
    await loadSources()
  } catch (err: any) {
    errorMsg.value = err?.message || '清空日志失败'
  }
}

function toggleFollow() {
  autoFollow.value = !autoFollow.value
  if (timer) {
    clearInterval(timer)
    timer = null
  }
  if (autoFollow.value) {
    fetchTail()
    timer = window.setInterval(fetchTail, 2500)
  }
}

onMounted(loadSources)
onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>

<template>
  <div class="flex flex-col h-[calc(100vh-6.5rem)] gap-3">
    <div class="flex flex-wrap items-center justify-between gap-2">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <ScrollText class="h-5 w-5" style="color: var(--zm-accent)" />
          日志中心
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          统一聚合系统、SSH、Nginx 与网站运行日志，支持实时追踪、关键字过滤与安全截断
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button class="zm-btn text-xs" :class="{ 'zm-btn-primary': autoFollow }" @click="toggleFollow">
          <Pause v-if="autoFollow" class="h-3.5 w-3.5" />
          <Play v-else class="h-3.5 w-3.5" />
          {{ autoFollow ? '实时追踪中 (2.5s)' : '开启实时追踪' }}
        </button>
        <button class="zm-btn text-xs" @click="loadSources(); fetchTail()">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
      </div>
    </div>

    <div class="flex-1 min-h-0 flex flex-col lg:flex-row gap-3">
      <!-- Left: Log Sources -->
      <div class="w-full lg:w-72 shrink-0 zm-card p-3 flex flex-col gap-2 overflow-y-auto">
        <div class="text-xs font-semibold">自动发现日志源</div>
        <button
          v-for="s in sources"
          :key="s.id"
          class="rounded-lg p-2.5 text-left text-xs border transition cursor-pointer"
          :style="{
            borderColor: activePath === s.path ? 'var(--zm-accent)' : 'var(--zm-border)',
            background: activePath === s.path ? 'var(--zm-accent-soft)' : 'var(--zm-surface-elevated)',
          }"
          @click="selectPath(s.path)"
        >
          <div class="flex items-center justify-between gap-1">
            <span class="font-medium truncate">{{ s.name }}</span>
            <span
              class="text-[10px] shrink-0"
              :style="{ color: s.exists ? 'var(--zm-accent)' : 'var(--zm-text-muted)' }"
            >
              {{ s.exists ? formatBytes(s.size_bytes) : '无文件' }}
            </span>
          </div>
          <div class="font-mono text-[10px] opacity-65 truncate mt-0.5">{{ s.path }}</div>
        </button>
      </div>

      <!-- Right: Tail Viewer -->
      <div class="flex-1 min-w-0 zm-card p-3 flex flex-col gap-2.5 overflow-hidden">
        <div class="flex flex-wrap items-center gap-2">
          <form class="flex-1 flex gap-1.5 min-w-[200px]" @submit.prevent="selectPath(customPath)">
            <input v-model="customPath" class="zm-input !py-1.5 font-mono text-xs" placeholder="输入任意日志路径回车查看" />
            <button type="submit" class="zm-btn !py-1.5 text-xs shrink-0">打开</button>
          </form>
          <div class="relative w-44">
            <Search class="h-3.5 w-3.5 absolute left-2.5 top-2 opacity-50" />
            <input
              v-model="keyword"
              class="zm-input !pl-8 !py-1.5 text-xs"
              placeholder="关键字过滤"
              @keyup.enter="fetchTail"
            />
          </div>
          <select v-model.number="lineCount" class="zm-input !w-auto !py-1.5 text-xs" @change="fetchTail">
            <option :value="100">最后 100 行</option>
            <option :value="300">最后 300 行</option>
            <option :value="1000">最后 1000 行</option>
          </select>
          <button class="zm-btn zm-btn-danger !py-1.5 text-xs" @click="truncateCurrent">
            <Trash2 class="h-3.5 w-3.5" /> 清空
          </button>
        </div>

        <div v-if="notice" class="text-xs" style="color: var(--zm-accent)">{{ notice }}</div>
        <div v-if="errorMsg" class="text-xs" style="color: var(--zm-danger)">{{ errorMsg }}</div>

        <pre
          class="flex-1 overflow-auto rounded-lg p-3 text-xs font-mono leading-relaxed m-0"
          style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
        >{{ content || '（日志内容为空或文件尚未生成）' }}</pre>

        <div class="flex items-center justify-between text-[11px]" style="color: var(--zm-text-muted)">
          <span class="font-mono truncate">{{ activePath }}</span>
          <span>文件大小 {{ formatBytes(fileSize) }} · 命中 {{ matchedLines }} 行</span>
        </div>
      </div>
    </div>
  </div>
</template>
