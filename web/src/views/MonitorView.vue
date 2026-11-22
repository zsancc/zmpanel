<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import * as echarts from 'echarts'
import { Activity, RefreshCw, Search, Skull } from 'lucide-vue-next'
import { apiRequest, formatBytes } from '@/api/client'
import { useThemeStore } from '@/stores/theme'

interface MetricPoint {
  timestamp: number
  cpu_usage: number
  mem_used_pct: number
  swap_used_pct: number
  load_1m: number
  net_rx_rate: number
  net_tx_rate: number
}

interface ProcessItem {
  pid: number
  name: string
  cpu_usage: number
  memory_bytes: number
  status: string
  cmd: string
}

const theme = useThemeStore()
const hours = ref(6)
const metrics = ref<MetricPoint[]>([])
const processes = ref<ProcessItem[]>([])
const sortBy = ref<'mem' | 'cpu' | 'pid'>('mem')
const searchKw = ref('')
const statusMsg = ref('')

const cpuChartEl = ref<HTMLDivElement | null>(null)
const netChartEl = ref<HTMLDivElement | null>(null)
let cpuChart: echarts.ECharts | null = null
let netChart: echarts.ECharts | null = null

async function loadMetrics() {
  try {
    metrics.value = await apiRequest<MetricPoint[]>(`/system/metrics?hours=${hours.value}&max_points=180`)
    renderCharts()
  } catch (err: any) {
    statusMsg.value = err?.message || '拉取历史指标失败'
  }
}

async function loadProcesses() {
  try {
    const q = encodeURIComponent(searchKw.value.trim())
    processes.value = await apiRequest<ProcessItem[]>(
      `/system/processes?sort=${sortBy.value}&limit=80${q ? `&q=${q}` : ''}`,
    )
  } catch (err: any) {
    statusMsg.value = err?.message || '拉取进程列表失败'
  }
}

async function killProc(p: ProcessItem) {
  if (!window.confirm(`确定强制结束进程 ${p.name} (PID: ${p.pid}) 吗？`)) return
  try {
    await apiRequest(`/system/processes/${p.pid}`, { method: 'DELETE' })
    statusMsg.value = `已结束进程 ${p.name} (PID: ${p.pid})`
    await loadProcesses()
  } catch (err: any) {
    statusMsg.value = err?.message || '结束进程失败'
  }
}

function renderCharts() {
  const dark = theme.isDark()
  const textColor = dark ? '#94a3b8' : '#64748b'
  const gridLine = dark ? 'rgba(148,163,184,0.12)' : 'rgba(15,23,42,0.08)'
  const times = metrics.value.map((m) =>
    new Date(m.timestamp * 1000).toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit' }),
  )

  if (cpuChartEl.value) {
    if (!cpuChart) cpuChart = echarts.init(cpuChartEl.value)
    cpuChart.setOption({
      tooltip: { trigger: 'axis' },
      legend: { data: ['CPU %', '内存 %', 'Swap %'], textStyle: { color: textColor }, top: 0 },
      grid: { left: 42, right: 16, top: 34, bottom: 26 },
      xAxis: { type: 'category', data: times, axisLabel: { color: textColor } },
      yAxis: { type: 'value', max: 100, axisLabel: { color: textColor, formatter: '{value}%' }, splitLine: { lineStyle: { color: gridLine } } },
      series: [
        { name: 'CPU %', type: 'line', smooth: true, showSymbol: false, data: metrics.value.map((m) => Number(m.cpu_usage.toFixed(1))), areaStyle: { opacity: 0.14 } },
        { name: '内存 %', type: 'line', smooth: true, showSymbol: false, data: metrics.value.map((m) => Number(m.mem_used_pct.toFixed(1))), areaStyle: { opacity: 0.1 } },
        { name: 'Swap %', type: 'line', smooth: true, showSymbol: false, data: metrics.value.map((m) => Number(m.swap_used_pct.toFixed(1))) },
      ],
    })
  }

  if (netChartEl.value) {
    if (!netChart) netChart = echarts.init(netChartEl.value)
    netChart.setOption({
      tooltip: { trigger: 'axis', valueFormatter: (v: any) => `${Number(v).toFixed(1)} KB/s` },
      legend: { data: ['下行 (KB/s)', '上行 (KB/s)'], textStyle: { color: textColor }, top: 0 },
      grid: { left: 52, right: 16, top: 34, bottom: 26 },
      xAxis: { type: 'category', data: times, axisLabel: { color: textColor } },
      yAxis: { type: 'value', axisLabel: { color: textColor }, splitLine: { lineStyle: { color: gridLine } } },
      series: [
        { name: '下行 (KB/s)', type: 'line', smooth: true, showSymbol: false, data: metrics.value.map((m) => Number((m.net_rx_rate / 1024).toFixed(1))), areaStyle: { opacity: 0.14 } },
        { name: '上行 (KB/s)', type: 'line', smooth: true, showSymbol: false, data: metrics.value.map((m) => Number((m.net_tx_rate / 1024).toFixed(1))), areaStyle: { opacity: 0.1 } },
      ],
    })
  }
}

function handleResize() {
  cpuChart?.resize()
  netChart?.resize()
}

watch(() => theme.themeId, () => nextTick(renderCharts))
watch(hours, loadMetrics)
watch(sortBy, loadProcesses)

onMounted(() => {
  loadMetrics()
  loadProcesses()
  window.addEventListener('resize', handleResize)
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', handleResize)
  cpuChart?.dispose()
  netChart?.dispose()
})
</script>

<template>
  <div class="space-y-5">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <Activity class="h-5 w-5" style="color: var(--zm-accent)" />
          系统监控与进程管理
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          SQLite 轻量降采样历史趋势（每 30 秒记录，自动保留 7 天）
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button
          v-for="h in [1, 6, 24, 168]"
          :key="h"
          class="zm-btn !py-1 !px-2.5 text-xs"
          :class="{ 'zm-btn-primary': hours === h }"
          @click="hours = h"
        >
          {{ h === 168 ? '7 天' : `${h} 小时` }}
        </button>
        <button class="zm-btn !py-1 !px-2.5 text-xs" @click="loadMetrics(); loadProcesses()">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
      </div>
    </div>

    <div v-if="statusMsg" class="zm-card p-3 text-xs">{{ statusMsg }}</div>

    <div class="grid grid-cols-1 xl:grid-cols-2 gap-4">
      <div class="zm-card p-4">
        <div class="text-sm font-semibold mb-2">CPU / 内存 / Swap 历史趋势</div>
        <div ref="cpuChartEl" class="h-64 w-full" />
      </div>
      <div class="zm-card p-4">
        <div class="text-sm font-semibold mb-2">网络带宽历史趋势</div>
        <div ref="netChartEl" class="h-64 w-full" />
      </div>
    </div>

    <!-- Process table -->
    <div class="zm-card p-4 space-y-3">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="text-sm font-semibold">进程列表 (TOP 80)</div>
        <div class="flex flex-wrap items-center gap-2">
          <div class="relative w-56">
            <Search class="h-3.5 w-3.5 absolute left-2.5 top-2.5 opacity-50" />
            <input
              v-model="searchKw"
              class="zm-input !pl-8 !py-1.5 text-xs"
              placeholder="搜索进程名 / PID / 命令"
              @keyup.enter="loadProcesses"
            />
          </div>
          <select v-model="sortBy" class="zm-input !w-auto !py-1.5 text-xs">
            <option value="mem">按内存排序</option>
            <option value="cpu">按 CPU 排序</option>
            <option value="pid">按 PID 排序</option>
          </select>
          <button class="zm-btn !py-1.5 text-xs" @click="loadProcesses">查询</button>
        </div>
      </div>

      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); color: var(--zm-text-muted)">
              <th class="py-2 px-2">PID</th>
              <th class="py-2 px-2">进程名</th>
              <th class="py-2 px-2">CPU</th>
              <th class="py-2 px-2">内存</th>
              <th class="py-2 px-2">状态</th>
              <th class="py-2 px-2">命令行</th>
              <th class="py-2 px-2 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="p in processes"
              :key="p.pid"
              class="border-b hover:bg-white/5"
              style="border-color: var(--zm-border)"
            >
              <td class="py-2 px-2 font-mono">{{ p.pid }}</td>
              <td class="py-2 px-2 font-medium">{{ p.name }}</td>
              <td class="py-2 px-2 font-mono">{{ p.cpu_usage.toFixed(1) }}%</td>
              <td class="py-2 px-2 font-mono">{{ formatBytes(p.memory_bytes) }}</td>
              <td class="py-2 px-2">{{ p.status }}</td>
              <td class="py-2 px-2 max-w-md truncate font-mono opacity-80" :title="p.cmd">
                {{ p.cmd || '-' }}
              </td>
              <td class="py-2 px-2 text-right">
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="killProc(p)">
                  <Skull class="h-3 w-3" /> 结束
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>
