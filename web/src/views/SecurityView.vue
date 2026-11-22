<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Ban, CheckCircle2, Lock, Plus, RefreshCw, ShieldAlert, Trash2, Unlock } from 'lucide-vue-next'
import { apiRequest, formatTimestamp } from '@/api/client'

interface SecurityConfig {
  enabled: boolean
  ssh_guard_enabled: boolean
  ssh_max_failures: number
  ssh_window_secs: number
  port_scan_enabled: boolean
  port_scan_threshold: number
  web_guard_enabled: boolean
  web_scan_threshold: number
  ban_duration_secs: number
  nginx_log_path: string
}

interface SecurityBanItem {
  ip: string
  reason: string
  category: string
  hits: number
  banned_at: number
  expires_at: number
}

interface SecurityEventItem {
  id: number
  timestamp: number
  category: string
  source_ip: string
  summary: string
  action_taken: string
  sample: string
}

interface WhitelistItem {
  ip: string
  remark: string
  created_at: number
}

interface SshHardeningStatus {
  config_path: string
  readable: boolean
  port: number
  password_auth: boolean
  permit_root_login: string
  pubkey_auth: boolean
}

const activeTab = ref<'bans' | 'events' | 'rules' | 'ssh'>('bans')
const config = ref<SecurityConfig>({
  enabled: true,
  ssh_guard_enabled: true,
  ssh_max_failures: 5,
  ssh_window_secs: 600,
  port_scan_enabled: true,
  port_scan_threshold: 8,
  web_guard_enabled: true,
  web_scan_threshold: 12,
  ban_duration_secs: 3600,
  nginx_log_path: '/var/log/nginx/access.log',
})
const bans = ref<SecurityBanItem[]>([])
const events = ref<SecurityEventItem[]>([])
const whitelist = ref<WhitelistItem[]>([])
const sshStatus = ref<SshHardeningStatus>({
  config_path: '/etc/ssh/sshd_config',
  readable: false,
  port: 22,
  password_auth: true,
  permit_root_login: 'prohibit-password',
  pubkey_auth: true,
})

const banForm = ref({ ip: '', reason: '', duration_secs: 86400 })
const whiteForm = ref({ ip: '', remark: '' })
const notice = ref('')
const errorMsg = ref('')

const CATEGORY_MAP: Record<string, string> = {
  ssh: 'SSH 爆破',
  port_scan: '端口扫描',
  web: '异常 Web',
  manual: '手动封禁',
  system: '系统',
}

async function loadOverview() {
  try {
    const res = await apiRequest<{
      config: SecurityConfig
      bans: SecurityBanItem[]
      events: SecurityEventItem[]
      whitelist: WhitelistItem[]
      ssh_hardening: SshHardeningStatus
    }>('/security/overview')
    config.value = res.config
    bans.value = res.bans
    events.value = res.events
    whitelist.value = res.whitelist
    sshStatus.value = res.ssh_hardening
  } catch (err: any) {
    errorMsg.value = err?.message || '加载安全中心数据失败'
  }
}

async function scanNow() {
  errorMsg.value = ''
  try {
    const count = await apiRequest<number>('/security/scan', { method: 'POST' })
    notice.value = `实时安全巡检完成，本次新增自动封禁 ${count} 个异常 IP`
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '巡检失败'
  }
}

async function manualBan() {
  if (!banForm.value.ip.trim()) return
  errorMsg.value = ''
  try {
    const msg = await apiRequest<string>('/security/bans', {
      method: 'POST',
      body: JSON.stringify(banForm.value),
    })
    banForm.value.ip = ''
    banForm.value.reason = ''
    notice.value = `封禁已生效：${msg}`
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '封禁失败'
  }
}

async function unban(ip: string) {
  errorMsg.value = ''
  try {
    const msg = await apiRequest<string>(`/security/bans/${encodeURIComponent(ip)}`, { method: 'DELETE' })
    notice.value = `已解封 ${ip}：${msg}`
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '解封失败'
  }
}

async function saveConfig() {
  errorMsg.value = ''
  try {
    await apiRequest('/security/config', {
      method: 'PUT',
      body: JSON.stringify(config.value),
    })
    notice.value = '内置安全引擎配置已保存并立即生效'
  } catch (err: any) {
    errorMsg.value = err?.message || '保存配置失败'
  }
}

async function addWhitelist() {
  if (!whiteForm.value.ip.trim()) return
  errorMsg.value = ''
  try {
    await apiRequest('/security/whitelist', {
      method: 'POST',
      body: JSON.stringify(whiteForm.value),
    })
    whiteForm.value = { ip: '', remark: '' }
    notice.value = '已加入安全白名单（该 IP 永不会被自动封禁）'
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '添加白名单失败'
  }
}

async function removeWhitelist(ip: string) {
  await apiRequest(`/security/whitelist/${encodeURIComponent(ip)}`, { method: 'DELETE' })
  await loadOverview()
}

async function applySsh() {
  if (!window.confirm('修改 SSH 配置前会自动备份 sshd_config 并执行 sshd -t 语法检查，确定应用吗？')) return
  errorMsg.value = ''
  try {
    const msg = await apiRequest<string>('/security/ssh', {
      method: 'PUT',
      body: JSON.stringify({
        port: Number(sshStatus.value.port),
        password_auth: sshStatus.value.password_auth,
        permit_root_login: sshStatus.value.permit_root_login,
        pubkey_auth: sshStatus.value.pubkey_auth,
      }),
    })
    notice.value = msg
    await loadOverview()
  } catch (err: any) {
    errorMsg.value = err?.message || '应用 SSH 加固配置失败'
  }
}

onMounted(loadOverview)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <ShieldAlert class="h-5 w-5" style="color: var(--zm-accent)" />
          安全中心（Rust 内置轻量防御引擎）
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          无需重型 Python fail2ban，&lt;2MB 内存常驻防御 SSH 爆破、端口扫描与异常 Web 探测
        </p>
      </div>
      <div class="flex gap-2">
        <button class="zm-btn text-xs" @click="loadOverview">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
        <button class="zm-btn zm-btn-primary text-xs" @click="scanNow">
          立即巡检一次
        </button>
      </div>
    </div>

    <!-- Summary KPI -->
    <div class="grid grid-cols-2 lg:grid-cols-4 gap-3 text-xs">
      <div class="zm-card p-3.5">
        <div style="color: var(--zm-text-muted)">引擎状态</div>
        <div class="text-base font-bold mt-1" :style="{ color: config.enabled ? 'var(--zm-accent)' : 'var(--zm-warning)' }">
          {{ config.enabled ? '实时防护中' : '已暂停' }}
        </div>
      </div>
      <div class="zm-card p-3.5">
        <div style="color: var(--zm-text-muted)">当前封禁 IP</div>
        <div class="text-base font-bold mt-1 text-rose-400">{{ bans.length }}</div>
      </div>
      <div class="zm-card p-3.5">
        <div style="color: var(--zm-text-muted)">近期拦截事件</div>
        <div class="text-base font-bold mt-1">{{ events.length }}</div>
      </div>
      <div class="zm-card p-3.5">
        <div style="color: var(--zm-text-muted)">白名单保护 IP</div>
        <div class="text-base font-bold mt-1 text-emerald-400">{{ whitelist.length }}</div>
      </div>
    </div>

    <div class="flex flex-wrap gap-2">
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'bans' }" @click="activeTab = 'bans'">
        <Ban class="h-3.5 w-3.5" /> 封禁列表 ({{ bans.length }})
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'events' }" @click="activeTab = 'events'">
        拦截事件时间线 ({{ events.length }})
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'rules' }" @click="activeTab = 'rules'">
        检测策略与白名单
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'ssh' }" @click="activeTab = 'ssh'">
        <Lock class="h-3.5 w-3.5" /> SSH 安全加固
      </button>
    </div>

    <div v-if="notice" class="zm-card p-3 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card p-3 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Tab 1: Bans -->
    <div v-if="activeTab === 'bans'" class="space-y-4">
      <div class="zm-card p-4 flex flex-wrap items-end gap-2 text-xs">
        <div class="w-44">
          <label class="block mb-1 opacity-75">手动拉黑 IP</label>
          <input v-model="banForm.ip" class="zm-input font-mono" placeholder="203.0.113.55" />
        </div>
        <div class="flex-1 min-w-[180px]">
          <label class="block mb-1 opacity-75">封禁原因</label>
          <input v-model="banForm.reason" class="zm-input" placeholder="恶意扫描 / 爆破" />
        </div>
        <div class="w-36">
          <label class="block mb-1 opacity-75">封禁时长</label>
          <select v-model.number="banForm.duration_secs" class="zm-input">
            <option :value="3600">1 小时</option>
            <option :value="86400">24 小时</option>
            <option :value="604800">7 天</option>
            <option :value="0">永久封禁</option>
          </select>
        </div>
        <button class="zm-btn zm-btn-danger text-xs" @click="manualBan">立即封禁</button>
      </div>

      <div class="zm-card overflow-hidden">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">IP 地址</th>
              <th class="py-2.5 px-2">攻击类型</th>
              <th class="py-2.5 px-2">命中次数</th>
              <th class="py-2.5 px-2">原因</th>
              <th class="py-2.5 px-2">解封时间</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="bans.length === 0">
              <td colspan="6" class="py-10 text-center opacity-60">当前没有被封禁的 IP</td>
            </tr>
            <tr v-for="b in bans" :key="b.ip" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
              <td class="py-2.5 px-3 font-mono font-semibold text-rose-400">{{ b.ip }}</td>
              <td class="py-2.5 px-2">{{ CATEGORY_MAP[b.category] || b.category }}</td>
              <td class="py-2.5 px-2 font-mono">{{ b.hits }}</td>
              <td class="py-2.5 px-2">{{ b.reason }}</td>
              <td class="py-2.5 px-2 opacity-75">{{ b.expires_at === 0 ? '永久' : formatTimestamp(b.expires_at) }}</td>
              <td class="py-2.5 px-3 text-right">
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="unban(b.ip)">
                  <Unlock class="h-3 w-3" /> 解封
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Tab 2: Events Timeline -->
    <div v-if="activeTab === 'events'" class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">时间</th>
              <th class="py-2.5 px-2">类型</th>
              <th class="py-2.5 px-2">来源 IP</th>
              <th class="py-2.5 px-2">告警摘要</th>
              <th class="py-2.5 px-2">处置</th>
              <th class="py-2.5 px-2">日志样本</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="events.length === 0">
              <td colspan="6" class="py-10 text-center opacity-60">暂无安全拦截记录</td>
            </tr>
            <tr v-for="e in events" :key="e.id" class="border-b" style="border-color: var(--zm-border)">
              <td class="py-2 px-3 whitespace-nowrap opacity-75">{{ formatTimestamp(e.timestamp) }}</td>
              <td class="py-2 px-2">{{ CATEGORY_MAP[e.category] || e.category }}</td>
              <td class="py-2 px-2 font-mono font-semibold">{{ e.source_ip }}</td>
              <td class="py-2 px-2">{{ e.summary }}</td>
              <td class="py-2 px-2 font-mono" style="color: var(--zm-accent)">{{ e.action_taken }}</td>
              <td class="py-2 px-2 font-mono max-w-sm truncate opacity-70" :title="e.sample">{{ e.sample || '-' }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Tab 3: Detection Policy & Whitelist -->
    <div v-if="activeTab === 'rules'" class="grid grid-cols-1 lg:grid-cols-2 gap-4 text-xs">
      <div class="zm-card p-5 space-y-3.5">
        <div class="font-semibold text-sm">自动检测策略（每 20 秒低开销巡检）</div>
        <label class="flex items-center gap-2 font-semibold cursor-pointer">
          <input v-model="config.enabled" type="checkbox" /> 启用内置轻量入侵防御引擎
        </label>
        <div class="grid grid-cols-2 gap-3 pt-1">
          <label class="flex items-center gap-2 cursor-pointer col-span-2">
            <input v-model="config.ssh_guard_enabled" type="checkbox" />
            SSH 防爆破（自动适配 journald / auth.log / Alpine messages）
          </label>
          <div>
            <label class="block mb-1 opacity-75">SSH 失败封禁阈值 (次)</label>
            <input v-model.number="config.ssh_max_failures" type="number" min="2" class="zm-input" />
          </div>
          <div>
            <label class="block mb-1 opacity-75">默认封禁时长 (秒，0=永久)</label>
            <input v-model.number="config.ban_duration_secs" type="number" min="0" class="zm-input" />
          </div>

          <label class="flex items-center gap-2 cursor-pointer col-span-2 pt-2">
            <input v-model="config.port_scan_enabled" type="checkbox" />
            端口扫描检测（解析内核防火墙丢弃包多端口探测）
          </label>
          <div>
            <label class="block mb-1 opacity-75">不同端口触发阈值 (个)</label>
            <input v-model.number="config.port_scan_threshold" type="number" min="3" class="zm-input" />
          </div>

          <label class="flex items-center gap-2 cursor-pointer col-span-2 pt-2">
            <input v-model="config.web_guard_enabled" type="checkbox" />
            异常 Web 访问拦截（.env / .git / wp-login / 高频 404）
          </label>
          <div>
            <label class="block mb-1 opacity-75">异常风险分阈值</label>
            <input v-model.number="config.web_scan_threshold" type="number" min="3" class="zm-input" />
          </div>
          <div>
            <label class="block mb-1 opacity-75">Nginx 访问日志路径</label>
            <input v-model="config.nginx_log_path" class="zm-input font-mono" />
          </div>
        </div>
        <button class="zm-btn zm-btn-primary text-xs" @click="saveConfig">
          <CheckCircle2 class="h-3.5 w-3.5" /> 保存防御策略
        </button>
      </div>

      <div class="zm-card p-5 space-y-3">
        <div class="font-semibold text-sm">IP 白名单（永不被自动封禁）</div>
        <div class="flex gap-2">
          <input v-model="whiteForm.ip" class="zm-input font-mono" placeholder="你的常用 IP（如 1.2.3.4）" />
          <input v-model="whiteForm.remark" class="zm-input" placeholder="备注（如家里宽带）" />
          <button class="zm-btn zm-btn-primary shrink-0" @click="addWhitelist">
            <Plus class="h-3.5 w-3.5" /> 添加
          </button>
        </div>
        <div class="space-y-2 pt-1">
          <div v-if="whitelist.length === 0" class="opacity-60">暂无白名单 IP</div>
          <div
            v-for="w in whitelist"
            :key="w.ip"
            class="flex items-center justify-between rounded-lg px-3 py-2"
            style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
          >
            <div>
              <span class="font-mono font-semibold">{{ w.ip }}</span>
              <span class="ml-2 opacity-70">{{ w.remark }}</span>
            </div>
            <button class="zm-btn zm-btn-danger !py-1 !px-2" @click="removeWhitelist(w.ip)">
              <Trash2 class="h-3 w-3" />
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Tab 4: SSH Hardening -->
    <div v-if="activeTab === 'ssh'" class="zm-card p-5 max-w-xl space-y-4 text-xs">
      <div class="font-semibold text-sm">SSH 服务安全加固 ({{ sshStatus.config_path }})</div>
      <div v-if="!sshStatus.readable" class="p-3 rounded-lg" style="background: rgba(245,158,11,0.12); color: var(--zm-warning)">
        当前未能读取 /etc/ssh/sshd_config（非 Linux 环境或权限不足），在 Linux 服务器以 root 运行面板时可直接修改并校验生效。
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="block mb-1 opacity-75">SSH 监听端口</label>
          <input v-model.number="sshStatus.port" type="number" min="1" max="65535" class="zm-input font-mono" />
        </div>
        <div>
          <label class="block mb-1 opacity-75">Root 登录策略 (PermitRootLogin)</label>
          <select v-model="sshStatus.permit_root_login" class="zm-input">
            <option value="prohibit-password">仅允许密钥登录 (prohibit-password 推荐)</option>
            <option value="no">完全禁止 Root 登录 (no)</option>
            <option value="yes">允许密码与密钥 (yes)</option>
          </select>
        </div>
      </div>
      <div class="space-y-2">
        <label class="flex items-center gap-2 cursor-pointer">
          <input v-model="sshStatus.pubkey_auth" type="checkbox" /> 允许 SSH 公钥认证 (PubkeyAuthentication)
        </label>
        <label class="flex items-center gap-2 cursor-pointer">
          <input v-model="sshStatus.password_auth" type="checkbox" /> 允许密码登录 (PasswordAuthentication，建议配置好密钥后关闭)
        </label>
      </div>
      <button class="zm-btn zm-btn-primary text-xs" @click="applySsh">
        校验 (sshd -t) 并应用加固配置
      </button>
    </div>
  </div>
</template>
