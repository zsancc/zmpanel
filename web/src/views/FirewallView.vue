<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Plus, RefreshCw, ShieldAlert, ShieldCheck, Trash2, X } from 'lucide-vue-next'
import { apiRequest, formatTimestamp } from '@/api/client'

interface FirewallStatus {
  backend: string
  active: boolean
  protected_ports: number[]
  ssh_port: number
  panel_port: number
  message: string
}

interface FirewallRule {
  id: string
  port: string
  protocol: string
  action: string
  source: string
  remark: string
  enabled: boolean
  created_at: number
}

const status = ref<FirewallStatus | null>(null)
const rules = ref<FirewallRule[]>([])
const notice = ref('')
const errorMsg = ref('')
const modalOpen = ref(false)
const form = ref({
  port: '',
  protocol: 'tcp',
  action: 'allow',
  source: 'any',
  remark: '',
})

async function loadOverview() {
  try {
    const res = await apiRequest<{ status: FirewallStatus; rules: FirewallRule[] }>('/firewall/overview')
    status.value = res.status
    rules.value = res.rules
  } catch (err: any) {
    errorMsg.value = err?.message || '加载防火墙信息失败'
  }
}

async function addRule() {
  errorMsg.value = ''
  notice.value = ''
  try {
    const msg = await apiRequest<string>('/firewall/rules', {
      method: 'POST',
      body: JSON.stringify(form.value),
    })
    modalOpen.value = false
    form.value = { port: '', protocol: 'tcp', action: 'allow', source: 'any', remark: '' }
    notice.value = `规则已添加：${msg}`
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '添加防火墙规则失败'
  }
}

async function removeRule(r: FirewallRule) {
  if (!window.confirm(`确定删除端口 ${r.port}/${r.protocol} (${r.action}) 的规则吗？`)) return
  errorMsg.value = ''
  try {
    const msg = await apiRequest<string>(`/firewall/rules/${r.id}`, { method: 'DELETE' })
    notice.value = `已删除规则：${msg}`
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '删除失败'
  }
}

async function syncNow() {
  errorMsg.value = ''
  try {
    const msg = await apiRequest<string>('/firewall/sync', { method: 'POST' })
    notice.value = msg
  } catch (err: any) {
    errorMsg.value = err?.message || '同步失败'
  }
}

onMounted(loadOverview)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <ShieldCheck class="h-5 w-5" style="color: var(--zm-accent)" />
          防火墙管理
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          优先原生操作 nftables（兼容 iptables / ufw），内置 SSH 与面板端口防自锁保护
        </p>
      </div>
      <div class="flex flex-wrap gap-2">
        <button class="zm-btn text-xs" @click="syncNow">
          <RefreshCw class="h-3.5 w-3.5" /> 立即同步至系统
        </button>
        <button class="zm-btn zm-btn-primary text-xs" @click="modalOpen = true">
          <Plus class="h-3.5 w-3.5" /> 添加端口规则
        </button>
      </div>
    </div>

    <!-- Status Banner -->
    <div v-if="status" class="zm-card p-4 flex flex-wrap items-center justify-between gap-3 text-xs">
      <div class="space-y-1">
        <div class="flex items-center gap-2 font-semibold">
          <span>底层引擎：</span>
          <span class="rounded px-2 py-0.5 font-mono" style="background: var(--zm-accent-soft); color: var(--zm-accent)">
            {{ status.backend }}
          </span>
        </div>
        <div style="color: var(--zm-text-muted)">{{ status.message }}</div>
      </div>
      <div
        class="flex items-center gap-2 rounded-lg px-3 py-2"
        style="background: rgba(16, 185, 129, 0.1); border: 1px solid rgba(16, 185, 129, 0.3)"
      >
        <ShieldAlert class="h-4 w-4 text-emerald-400 shrink-0" />
        <div>
          <div class="font-semibold text-emerald-400">防自锁保护已生效</div>
          <div class="opacity-80 font-mono">
            受保护端口: SSH {{ status.ssh_port }} / 面板 {{ status.panel_port }}
          </div>
        </div>
      </div>
    </div>

    <div v-if="notice" class="zm-card p-3 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card p-3 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Rules Table -->
    <div class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">端口 / 范围</th>
              <th class="py-2.5 px-2">协议</th>
              <th class="py-2.5 px-2">策略</th>
              <th class="py-2.5 px-2">来源 IP / CIDR</th>
              <th class="py-2.5 px-2">备注</th>
              <th class="py-2.5 px-2">创建时间</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="rules.length === 0">
              <td colspan="7" class="py-10 text-center opacity-60">暂无自定义端口规则，可点击右上角添加</td>
            </tr>
            <tr v-for="r in rules" :key="r.id" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
              <td class="py-2.5 px-3 font-mono font-semibold">{{ r.port }}</td>
              <td class="py-2.5 px-2 uppercase font-mono">{{ r.protocol }}</td>
              <td class="py-2.5 px-2">
                <span
                  class="rounded px-2 py-0.5 text-[11px] font-semibold"
                  :style="
                    r.action === 'allow'
                      ? { background: 'rgba(16,185,129,0.14)', color: '#10b981' }
                      : { background: 'rgba(239,68,68,0.14)', color: '#ef4444' }
                  "
                >
                  {{ r.action === 'allow' ? '放行 (ALLOW)' : '拦截 (DENY)' }}
                </span>
              </td>
              <td class="py-2.5 px-2 font-mono">{{ r.source }}</td>
              <td class="py-2.5 px-2">{{ r.remark || '-' }}</td>
              <td class="py-2.5 px-2 opacity-70">{{ formatTimestamp(r.created_at) }}</td>
              <td class="py-2.5 px-3 text-right">
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="removeRule(r)">
                  <Trash2 class="h-3 w-3" /> 删除
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Add Rule Modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4">
      <div class="zm-card w-full max-w-md p-5 space-y-3 text-xs">
        <div class="flex items-center justify-between font-semibold text-sm">
          <span>新增防火墙端口规则</span>
          <button @click="modalOpen = false"><X class="h-4 w-4" /></button>
        </div>
        <div>
          <label class="block mb-1 opacity-75">端口或范围（例如 80、443 或 8000-8010）</label>
          <input v-model="form.port" class="zm-input font-mono" placeholder="443" />
        </div>
        <div class="grid grid-cols-2 gap-2">
          <div>
            <label class="block mb-1 opacity-75">协议</label>
            <select v-model="form.protocol" class="zm-input">
              <option value="tcp">TCP</option>
              <option value="udp">UDP</option>
              <option value="both">TCP + UDP</option>
            </select>
          </div>
          <div>
            <label class="block mb-1 opacity-75">动作</label>
            <select v-model="form.action" class="zm-input">
              <option value="allow">放行 (allow)</option>
              <option value="deny">拦截 (deny)</option>
            </select>
          </div>
        </div>
        <div>
          <label class="block mb-1 opacity-75">来源 IP / CIDR（留空或 any 表示所有来源）</label>
          <input v-model="form.source" class="zm-input font-mono" placeholder="any" />
        </div>
        <div>
          <label class="block mb-1 opacity-75">备注</label>
          <input v-model="form.remark" class="zm-input" placeholder="例如：HTTPS Web 服务" />
        </div>
        <div class="flex justify-end gap-2 pt-2">
          <button class="zm-btn" @click="modalOpen = false">取消</button>
          <button class="zm-btn zm-btn-primary" @click="addRule">保存并生效</button>
        </div>
      </div>
    </div>
  </div>
</template>
