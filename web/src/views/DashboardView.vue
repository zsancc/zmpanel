<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  ArrowDownUp,
  Cpu,
  FolderOpen,
  HardDrive,
  MemoryStick,
  RefreshCw,
  Server,
  Terminal,
  Zap,
} from 'lucide-vue-next'
import { apiRequest, formatBytes, formatDuration } from '@/api/client'

interface DiskInfo {
  name: string
  mount_point: string
  fs_type: string
  total_bytes: number
  available_bytes: number
  used_bytes: number
}

interface RuntimeEnvInfo {
  virt_type: string
  virt_label: string
  inside_container: boolean
  docker_nesting_supported: boolean
  docker_mode: string
  docker_nesting_reason: string
}

interface SystemSnapshot {
  timestamp: number
  hostname: string
  os_name: string
  os_version: string
  kernel_version: string
  arch: string
  uptime_secs: number
  load_avg: [number, number, number]
  cpu_brand: string
  cpu_cores: number
  cpu_usage: number
  mem_total: number
  mem_used: number
  mem_available: number
  swap_total: number
  swap_used: number
  net_rx_rate: number
  net_tx_rate: number
  net_rx_month: number
  net_tx_month: number
  disks: DiskInfo[]
  panel_rss_bytes: number
  runtime_env?: RuntimeEnvInfo
}

const router = useRouter()
const snapshot = ref<SystemSnapshot | null>(null)
const errorMsg = ref('')
const swapModalOpen = ref(false)
const swapSizeMb = ref(512)
const swapBusy = ref(false)
const swapNotice = ref('')
let timer: number | null = null

const memPct = computed(() => {
  if (!snapshot.value || !snapshot.value.mem_total) return 0
  return Math.min(100, (snapshot.value.mem_used / snapshot.value.mem_total) * 100)
})

const swapPct = computed(() => {
  if (!snapshot.value || !snapshot.value.swap_total) return 0
  return Math.min(100, (snapshot.value.swap_used / snapshot.value.swap_total) * 100)
})

async function fetchSnapshot() {
  try {
    snapshot.value = await apiRequest<SystemSnapshot>('/system/snapshot')
    errorMsg.value = ''
  } catch (err: any) {
    errorMsg.value = err?.message || '拉取监控数据失败'
  }
}

async function applySwap() {
  swapBusy.value = true
  swapNotice.value = ''
  try {
    const msg = await apiRequest<string>('/system/swap', {
      method: 'POST',
      body: JSON.stringify({ size_mb: Number(swapSizeMb.value) }),
    })
    swapNotice.value = msg
    await fetchSnapshot()
  } catch (err: any) {
    swapNotice.value = err?.message || '配置 Swap 失败'
  } finally {
    swapBusy.value = false
  }
}

onMounted(() => {
  fetchSnapshot()
  timer = window.setInterval(fetchSnapshot, 2500)
})

onBeforeUnmount(() => {
  if (timer) clearInterval(timer)
})
</script>

<template>
  <div class="space-y-5">
    <!-- Header -->
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold">系统概览</h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          {{ snapshot ? `${snapshot.hostname} · ${snapshot.os_name} ${snapshot.os_version} (${snapshot.arch})` : '正在采集节点实时指标...' }}
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button class="zm-btn text-xs" @click="router.push('/files')">
          <FolderOpen class="h-3.5 w-3.5" /> 文件管理
        </button>
        <button class="zm-btn text-xs" @click="router.push('/terminal')">
          <Terminal class="h-3.5 w-3.5" /> 打开终端
        </button>
        <button class="zm-btn text-xs" @click="fetchSnapshot">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
      </div>
    </div>

    <div
      v-if="errorMsg"
      class="zm-card p-3 text-xs"
      style="border-color: var(--zm-danger); color: var(--zm-danger)"
    >
      {{ errorMsg }}
    </div>

    <!-- Top Metric Cards -->
    <div v-if="snapshot" class="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-4">
      <!-- CPU -->
      <div class="zm-card p-4 space-y-3">
        <div class="flex items-center justify-between">
          <span class="text-xs font-medium" style="color: var(--zm-text-muted)">CPU 使用率</span>
          <Cpu class="h-4 w-4" style="color: var(--zm-accent)" />
        </div>
        <div class="flex items-baseline justify-between">
          <span class="text-2xl font-bold">{{ snapshot.cpu_usage.toFixed(1) }}%</span>
          <span class="text-xs" style="color: var(--zm-text-muted)">{{ snapshot.cpu_cores }} 核心</span>
        </div>
        <div class="h-2 rounded-full overflow-hidden" style="background: var(--zm-surface-elevated)">
          <div
            class="h-full rounded-full transition-all duration-500"
            :style="{ width: `${Math.min(100, snapshot.cpu_usage)}%`, background: 'var(--zm-accent)' }"
          />
        </div>
        <div class="text-[11px] truncate" style="color: var(--zm-text-muted)">
          {{ snapshot.cpu_brand }}
        </div>
      </div>

      <!-- Memory -->
      <div class="zm-card p-4 space-y-3">
        <div class="flex items-center justify-between">
          <span class="text-xs font-medium" style="color: var(--zm-text-muted)">物理内存</span>
          <MemoryStick class="h-4 w-4" style="color: var(--zm-info)" />
        </div>
        <div class="flex items-baseline justify-between">
          <span class="text-2xl font-bold">{{ memPct.toFixed(1) }}%</span>
          <span class="text-xs" style="color: var(--zm-text-muted)">
            {{ formatBytes(snapshot.mem_used) }} / {{ formatBytes(snapshot.mem_total) }}
          </span>
        </div>
        <div class="h-2 rounded-full overflow-hidden" style="background: var(--zm-surface-elevated)">
          <div
            class="h-full rounded-full transition-all duration-500"
            :style="{ width: `${memPct}%`, background: memPct > 85 ? 'var(--zm-danger)' : 'var(--zm-info)' }"
          />
        </div>
        <div class="flex items-center justify-between text-[11px]" style="color: var(--zm-text-muted)">
          <span>面板自身占用: <strong style="color: var(--zm-accent)">{{ formatBytes(snapshot.panel_rss_bytes) }}</strong></span>
        </div>
      </div>

      <!-- Swap -->
      <div class="zm-card p-4 space-y-3">
        <div class="flex items-center justify-between">
          <span class="text-xs font-medium" style="color: var(--zm-text-muted)">Swap 虚拟内存</span>
          <button
            class="text-[11px] font-medium underline cursor-pointer"
            style="color: var(--zm-accent)"
            @click="swapModalOpen = true"
          >
            管理 Swap
          </button>
        </div>
        <div class="flex items-baseline justify-between">
          <span class="text-2xl font-bold">{{ swapPct.toFixed(1) }}%</span>
          <span class="text-xs" style="color: var(--zm-text-muted)">
            {{ formatBytes(snapshot.swap_used) }} / {{ formatBytes(snapshot.swap_total) }}
          </span>
        </div>
        <div class="h-2 rounded-full overflow-hidden" style="background: var(--zm-surface-elevated)">
          <div
            class="h-full rounded-full transition-all duration-500"
            :style="{ width: `${swapPct}%`, background: 'var(--zm-warning)' }"
          />
        </div>
        <div class="text-[11px]" style="color: var(--zm-text-muted)">
          256MB 小鸡建议配置 512MB~1GB Swap 防 OOM
        </div>
      </div>

      <!-- Network & Monthly Traffic -->
      <div class="zm-card p-4 space-y-3">
        <div class="flex items-center justify-between">
          <span class="text-xs font-medium" style="color: var(--zm-text-muted)">网络实时速率</span>
          <ArrowDownUp class="h-4 w-4" style="color: var(--zm-accent)" />
        </div>
        <div class="grid grid-cols-2 gap-2 text-xs">
          <div class="rounded-lg p-2" style="background: var(--zm-surface-elevated)">
            <div style="color: var(--zm-text-muted)">下行 ↓</div>
            <div class="font-bold text-sm mt-0.5">{{ formatBytes(snapshot.net_rx_rate) }}/s</div>
          </div>
          <div class="rounded-lg p-2" style="background: var(--zm-surface-elevated)">
            <div style="color: var(--zm-text-muted)">上行 ↑</div>
            <div class="font-bold text-sm mt-0.5">{{ formatBytes(snapshot.net_tx_rate) }}/s</div>
          </div>
        </div>
        <div class="flex items-center justify-between text-[11px]" style="color: var(--zm-text-muted)">
          <span>本月累计: ↓ {{ formatBytes(snapshot.net_rx_month) }}</span>
          <span>↑ {{ formatBytes(snapshot.net_tx_month) }}</span>
        </div>
      </div>
    </div>

    <!-- System Info + Disks -->
    <div v-if="snapshot" class="grid grid-cols-1 lg:grid-cols-3 gap-4">
      <div class="zm-card p-5 space-y-3">
        <div class="flex items-center gap-2 font-semibold text-sm">
          <Server class="h-4 w-4" style="color: var(--zm-accent)" />
          系统信息
        </div>
        <div class="space-y-2 text-xs">
          <div class="flex justify-between py-1.5 border-b" style="border-color: var(--zm-border)">
            <span style="color: var(--zm-text-muted)">主机名</span>
            <span class="font-medium">{{ snapshot.hostname }}</span>
          </div>
          <div class="flex justify-between py-1.5 border-b" style="border-color: var(--zm-border)">
            <span style="color: var(--zm-text-muted)">操作系统</span>
            <span class="font-medium">{{ snapshot.os_name }} {{ snapshot.os_version }}</span>
          </div>
          <div class="flex justify-between py-1.5 border-b" style="border-color: var(--zm-border)">
            <span style="color: var(--zm-text-muted)">内核版本</span>
            <span class="font-medium truncate max-w-[180px]">{{ snapshot.kernel_version }}</span>
          </div>
          <div class="flex justify-between py-1.5 border-b" style="border-color: var(--zm-border)">
            <span style="color: var(--zm-text-muted)">系统架构</span>
            <span class="font-medium">{{ snapshot.arch }}</span>
          </div>
          <div
            v-if="snapshot.runtime_env"
            class="flex justify-between items-center gap-2 py-1.5 border-b"
            style="border-color: var(--zm-border)"
          >
            <span style="color: var(--zm-text-muted)">运行环境</span>
            <span class="font-medium flex items-center gap-1.5 text-right">
              <span>{{ snapshot.runtime_env.virt_label }}</span>
              <span
                v-if="!snapshot.runtime_env.docker_nesting_supported"
                class="rounded px-1.5 py-0.5 text-[10px]"
                style="background: rgba(239,68,68,0.14); color: var(--zm-danger)"
                :title="snapshot.runtime_env.docker_nesting_reason"
              >
                不可套娃 Docker
              </span>
            </span>
          </div>
          <div class="flex justify-between py-1.5 border-b" style="border-color: var(--zm-border)">
            <span style="color: var(--zm-text-muted)">平均负载 (1/5/15m)</span>
            <span class="font-medium">
              {{ snapshot.load_avg[0].toFixed(2) }} / {{ snapshot.load_avg[1].toFixed(2) }} / {{ snapshot.load_avg[2].toFixed(2) }}
            </span>
          </div>
          <div class="flex justify-between py-1.5">
            <span style="color: var(--zm-text-muted)">已运行时长</span>
            <span class="font-medium">{{ formatDuration(snapshot.uptime_secs) }}</span>
          </div>
        </div>
      </div>

      <div class="zm-card p-5 lg:col-span-2 space-y-3">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 font-semibold text-sm">
            <HardDrive class="h-4 w-4" style="color: var(--zm-accent)" />
            存储分区
          </div>
          <span class="text-xs" style="color: var(--zm-text-muted)">共 {{ snapshot.disks.length }} 个挂载点</span>
        </div>
        <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
          <div
            v-for="d in snapshot.disks"
            :key="d.mount_point + d.name"
            class="rounded-lg p-3 space-y-2"
            style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
          >
            <div class="flex items-center justify-between text-xs">
              <span class="font-semibold truncate">{{ d.mount_point }}</span>
              <span class="px-1.5 py-0.5 rounded text-[10px]" style="background: var(--zm-surface)">
                {{ d.fs_type }}
              </span>
            </div>
            <div class="h-1.5 rounded-full overflow-hidden" style="background: var(--zm-border)">
              <div
                class="h-full rounded-full"
                :style="{
                  width: `${d.total_bytes ? Math.min(100, (d.used_bytes / d.total_bytes) * 100) : 0}%`,
                  background: d.total_bytes && d.used_bytes / d.total_bytes > 0.88 ? 'var(--zm-danger)' : 'var(--zm-accent)',
                }"
              />
            </div>
            <div class="flex justify-between text-[11px]" style="color: var(--zm-text-muted)">
              <span>已用 {{ formatBytes(d.used_bytes) }}</span>
              <span>总计 {{ formatBytes(d.total_bytes) }} (可用 {{ formatBytes(d.available_bytes) }})</span>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- Swap Dialog -->
    <div
      v-if="swapModalOpen"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
    >
      <div class="zm-card w-full max-w-md p-5 space-y-4 shadow-2xl">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 font-semibold">
            <Zap class="h-4 w-4" style="color: var(--zm-accent)" />
            Linux Swap 虚拟内存配置
          </div>
          <button class="text-xs opacity-60 hover:opacity-100 cursor-pointer" @click="swapModalOpen = false">
            关闭
          </button>
        </div>
        <p class="text-xs" style="color: var(--zm-text-muted)">
          针对 256MB / 512MB NAT 小鸡，一键创建或调整 <code>/swapfile.zmpanel</code>（填 0 表示关闭并删除）。
        </p>
        <div
          v-if="snapshot?.runtime_env?.inside_container"
          class="text-[11px] p-2.5 rounded-lg"
          style="background: rgba(245,158,11,0.12); color: var(--zm-warning)"
        >
          提示：当前检测到运行在「{{ snapshot.runtime_env.virt_label }}」中，宿主机通常会限制容器内核执行 <code>swapon</code>，若开启失败属宿主机限制。
        </div>
        <div class="flex gap-2">
          <button
            v-for="preset in [256, 512, 1024, 2048, 0]"
            :key="preset"
            class="zm-btn !px-2.5 !py-1 text-xs"
            @click="swapSizeMb = preset"
          >
            {{ preset === 0 ? '关闭' : `${preset}MB` }}
          </button>
        </div>
        <div>
          <label class="text-xs block mb-1" style="color: var(--zm-text-muted)">Swap 大小 (MB)</label>
          <input v-model.number="swapSizeMb" type="number" min="0" max="16384" class="zm-input" />
        </div>
        <div v-if="swapNotice" class="text-xs p-2.5 rounded-lg" style="background: var(--zm-surface-elevated)">
          {{ swapNotice }}
        </div>
        <div class="flex justify-end gap-2">
          <button class="zm-btn" @click="swapModalOpen = false">取消</button>
          <button class="zm-btn zm-btn-primary" :disabled="swapBusy" @click="applySwap">
            {{ swapBusy ? '执行中...' : '立即应用' }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>
