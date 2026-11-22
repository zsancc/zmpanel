<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Clock, Play, Plus, RefreshCw, Trash2, X } from 'lucide-vue-next'
import { apiRequest, formatTimestamp } from '@/api/client'

interface CronJobItem {
  id: string
  name: string
  kind: string
  schedule: string
  target: string
  enabled: boolean
  keep_count: number
  last_run_at: number
  last_status: string
}

interface CronRunRecord {
  id: number
  job_id: string
  job_name: string
  started_at: number
  duration_ms: number
  success: boolean
  output: string
}

const KIND_LABELS: Record<string, string> = {
  backup: '定时自动备份',
  shell: 'Shell 脚本',
  http: 'HTTP URL 探测',
  cleanup: '日志与回收站清理',
  site_restart: '定时重启网站',
}

const SCHEDULE_PRESETS = [
  { label: '每 5 分钟', expr: '*/5 * * * *' },
  { label: '每小时整点', expr: '0 * * * *' },
  { label: '每天凌晨 3:00', expr: '0 3 * * *' },
  { label: '每周日凌晨 4:00', expr: '0 4 * * 0' },
]

const jobs = ref<CronJobItem[]>([])
const runs = ref<CronRunRecord[]>([])
const notice = ref('')
const errorMsg = ref('')
const busyId = ref('')
const modalOpen = ref(false)
const form = ref({
  id: '' as string | undefined,
  name: '',
  kind: 'backup',
  schedule: '0 3 * * *',
  target: '',
  enabled: true,
  keep_count: 5,
})

async function loadAll() {
  try {
    const [j, r] = await Promise.all([
      apiRequest<CronJobItem[]>('/cron/jobs'),
      apiRequest<CronRunRecord[]>('/cron/runs?limit=40'),
    ])
    jobs.value = j
    runs.value = r
  } catch (err: any) {
    errorMsg.value = err?.message || '加载计划任务失败'
  }
}

function openCreate() {
  form.value = {
    id: undefined,
    name: '',
    kind: 'backup',
    schedule: '0 3 * * *',
    target: '',
    enabled: true,
    keep_count: 5,
  }
  modalOpen.value = true
}

function openEdit(job: CronJobItem) {
  form.value = {
    id: job.id,
    name: job.name,
    kind: job.kind,
    schedule: job.schedule,
    target: job.target,
    enabled: job.enabled,
    keep_count: job.keep_count,
  }
  modalOpen.value = true
}

async function saveJob() {
  errorMsg.value = ''
  try {
    await apiRequest('/cron/jobs', {
      method: 'POST',
      body: JSON.stringify(form.value),
    })
    modalOpen.value = false
    notice.value = '计划任务已保存并加入后台调度'
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '保存计划任务失败'
  }
}

async function runNow(job: CronJobItem) {
  busyId.value = job.id
  errorMsg.value = ''
  try {
    const res = await apiRequest<{ success: boolean; output: string }>(`/cron/jobs/${job.id}/run`, {
      method: 'POST',
    })
    notice.value = `${job.name} 执行${res.success ? '成功' : '失败'}：${res.output.slice(0, 180)}`
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '执行任务失败'
  } finally {
    busyId.value = ''
  }
}

async function deleteJob(job: CronJobItem) {
  if (!window.confirm(`确定删除计划任务「${job.name}」吗？`)) return
  await apiRequest(`/cron/jobs/${job.id}`, { method: 'DELETE' })
  await loadAll()
}

onMounted(loadAll)
</script>

<template>
  <div class="space-y-5">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <Clock class="h-5 w-5" style="color: var(--zm-accent)" />
          计划任务 (Cron)
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          纯 Rust 内置 Cron 调度器：支持定时自动备份与份数轮转、脚本、HTTP 保活探测、清理与站点重启
        </p>
      </div>
      <div class="flex gap-2">
        <button class="zm-btn text-xs" @click="loadAll">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
        <button class="zm-btn zm-btn-primary text-xs" @click="openCreate">
          <Plus class="h-3.5 w-3.5" /> 新增计划任务
        </button>
      </div>
    </div>

    <div v-if="notice" class="zm-card p-3 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card p-3 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Jobs Table -->
    <div class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">任务名称</th>
              <th class="py-2.5 px-2">类型</th>
              <th class="py-2.5 px-2">Cron 周期</th>
              <th class="py-2.5 px-2">状态</th>
              <th class="py-2.5 px-2">最近执行</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="jobs.length === 0">
              <td colspan="6" class="py-10 text-center opacity-60">暂无计划任务，推荐添加一个每日定时备份任务</td>
            </tr>
            <tr v-for="j in jobs" :key="j.id" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
              <td class="py-2.5 px-3">
                <div class="font-semibold">{{ j.name }}</div>
                <div class="font-mono text-[10px] opacity-65 max-w-xs truncate">{{ j.target || '全部网站/Docker/SSL/面板库' }}</div>
              </td>
              <td class="py-2.5 px-2">{{ KIND_LABELS[j.kind] || j.kind }}</td>
              <td class="py-2.5 px-2 font-mono">{{ j.schedule }}</td>
              <td class="py-2.5 px-2">
                <span :class="j.enabled ? 'text-emerald-400' : 'opacity-50'">
                  {{ j.enabled ? '● 启用' : '○ 暂停' }}
                </span>
              </td>
              <td class="py-2.5 px-2">
                <div>{{ j.last_run_at ? formatTimestamp(j.last_run_at) : '尚未执行' }}</div>
                <div v-if="j.last_status" class="text-[10px]" :class="j.last_status === '成功' ? 'text-emerald-400' : 'text-rose-400'">
                  {{ j.last_status }}
                </div>
              </td>
              <td class="py-2.5 px-3 text-right space-x-1 whitespace-nowrap">
                <button class="zm-btn zm-btn-primary !py-1 !px-2 text-[11px]" :disabled="busyId === j.id" @click="runNow(j)">
                  <Play class="h-3 w-3" /> {{ busyId === j.id ? '执行中' : '立即执行' }}
                </button>
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="openEdit(j)">编辑</button>
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="deleteJob(j)">
                  <Trash2 class="h-3 w-3" />
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Execution History -->
    <div class="zm-card p-4 space-y-3">
      <div class="font-semibold text-sm">最近执行历史 (40 条)</div>
      <div v-if="runs.length === 0" class="text-xs opacity-60">暂无执行记录</div>
      <div v-else class="space-y-2 max-h-96 overflow-y-auto">
        <div
          v-for="r in runs"
          :key="r.id"
          class="rounded-lg p-3 text-xs space-y-1.5"
          style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
        >
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div class="flex items-center gap-2">
              <span class="font-semibold">{{ r.job_name }}</span>
              <span
                class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
                :style="
                  r.success
                    ? { background: 'rgba(16,185,129,0.14)', color: '#10b981' }
                    : { background: 'rgba(239,68,68,0.14)', color: '#ef4444' }
                "
              >
                {{ r.success ? '成功' : '失败' }}
              </span>
            </div>
            <span class="text-[11px] opacity-65">
              {{ formatTimestamp(r.started_at) }} · 耗时 {{ r.duration_ms }}ms
            </span>
          </div>
          <pre
            class="p-2 rounded text-[11px] font-mono overflow-auto max-h-32 m-0"
            style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
          >{{ r.output }}</pre>
        </div>
      </div>
    </div>

    <!-- Create / Edit Modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4">
      <div class="zm-card w-full max-w-lg p-5 space-y-3 text-xs">
        <div class="flex items-center justify-between font-semibold text-sm">
          <span>{{ form.id ? '编辑计划任务' : '新增计划任务' }}</span>
          <button @click="modalOpen = false"><X class="h-4 w-4" /></button>
        </div>
        <div>
          <label class="block mb-1 opacity-75">任务名称</label>
          <input v-model="form.name" class="zm-input" placeholder="例如：每日全站自动备份" />
        </div>
        <div class="grid grid-cols-2 gap-2">
          <div>
            <label class="block mb-1 opacity-75">任务类型</label>
            <select v-model="form.kind" class="zm-input">
              <option value="backup">定时自动备份（含份数轮转）</option>
              <option value="shell">Shell 脚本 / 命令</option>
              <option value="http">HTTP URL 探测 / Webhook</option>
              <option value="cleanup">清理回收站与大日志</option>
              <option value="site_restart">定时重启指定网站</option>
            </select>
          </div>
          <div>
            <label class="block mb-1 opacity-75">保留备份份数 (backup 类型)</label>
            <input v-model.number="form.keep_count" type="number" min="1" max="100" class="zm-input" />
          </div>
        </div>
        <div>
          <label class="block mb-1 opacity-75">Cron 表达式（分 时 日 月 周）</label>
          <input v-model="form.schedule" class="zm-input font-mono" placeholder="0 3 * * *" />
          <div class="flex flex-wrap gap-1.5 mt-1.5">
            <button
              v-for="p in SCHEDULE_PRESETS"
              :key="p.expr"
              type="button"
              class="zm-btn !py-0.5 !px-2 text-[11px]"
              @click="form.schedule = p.expr"
            >
              {{ p.label }}
            </button>
          </div>
        </div>
        <div v-if="form.kind !== 'cleanup'">
          <label class="block mb-1 opacity-75">
            {{
              form.kind === 'shell'
                ? '脚本内容'
                : form.kind === 'http'
                  ? '请求 URL (http:// 或 https://)'
                  : form.kind === 'site_restart'
                    ? '网站 ID'
                    : '备份范围 JSON（留空默认备份全部网站 + Docker + SSL + 面板库）'
            }}
          </label>
          <textarea v-model="form.target" rows="3" class="zm-input font-mono" />
        </div>
        <label class="flex items-center gap-2 cursor-pointer">
          <input v-model="form.enabled" type="checkbox" /> 启用该定时任务
        </label>
        <div class="flex justify-end gap-2 pt-2">
          <button class="zm-btn" @click="modalOpen = false">取消</button>
          <button class="zm-btn zm-btn-primary" @click="saveJob">保存任务</button>
        </div>
      </div>
    </div>
  </div>
</template>
