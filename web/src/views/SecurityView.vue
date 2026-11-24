<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
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

interface VulnPackageItem {
  package: string
  category: string
  severity: string
  current_version: string
  candidate_version: string
  is_security_repo: boolean
  advisory: string
  cves?: string[]
}

interface SecurityBaselineCheck {
  id: string
  category: string
  title: string
  severity: string
  status: string
  detail: string
  remediation: string
  cves?: string[]
}

interface VulnWatchConfig {
  enabled: boolean
  interval_hours: number
  alert_min_severity: string
  auto_fix_non_kernel: boolean
  last_scan_at: number
  last_alert_at: number
  last_summary: string
}

interface VulnScanReport {
  scanned_at: number
  os_family: string
  kernel_running: string
  reboot_required: boolean
  security_score: number
  critical_count: number
  high_count: number
  total_upgradable: number
  packages: VulnPackageItem[]
  baselines: SecurityBaselineCheck[]
  summary: string
}

const router = useRouter()
const activeTab = ref<'bans' | 'vuln' | 'events' | 'rules' | 'ssh'>('bans')
const vulnReport = ref<VulnScanReport | null>(null)
const vulnWatch = ref<VulnWatchConfig>({
  enabled: false,
  interval_hours: 6,
  alert_min_severity: 'high',
  auto_fix_non_kernel: false,
  last_scan_at: 0,
  last_alert_at: 0,
  last_summary: '',
})
const vulnScanning = ref(false)
const vulnFixing = ref(false)
const selectedPkgs = ref<string[]>([])
const fixOutput = ref('')
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

const VULN_CAT_MAP: Record<string, string> = {
  kernel: 'Linux 内核',
  privilege: '提权/核心库',
  ssh: 'OpenSSH',
  web: 'Nginx / TLS',
  runtime: 'Node.js / 运行时',
  container: 'Docker 容器',
  database: '数据库',
  network: '网络暴露',
  system: '系统包',
}

const SEV_MAP: Record<string, { label: string; color: string; bg: string }> = {
  critical: { label: '严重', color: '#f43f5e', bg: 'rgba(244,63,94,0.14)' },
  high: { label: '高危', color: '#f97316', bg: 'rgba(249,115,22,0.14)' },
  medium: { label: '中危', color: '#eab308', bg: 'rgba(234,179,8,0.14)' },
  low: { label: '低危', color: '#64748b', bg: 'rgba(100,116,139,0.14)' },
  info: { label: '提示', color: '#38bdf8', bg: 'rgba(56,189,248,0.14)' },
  pass: { label: '通过', color: '#10b981', bg: 'rgba(16,185,129,0.14)' },
}

function cveUrl(id: string): string {
  if (id.startsWith('CVE-')) return `https://nvd.nist.gov/vuln/detail/${encodeURIComponent(id)}`
  if (id.startsWith('DSA-')) return `https://security-tracker.debian.org/tracker/${encodeURIComponent(id)}`
  if (id.startsWith('USN-')) return `https://ubuntu.com/security/notices/${encodeURIComponent(id)}`
  return `https://nvd.nist.gov/vuln/search/results?query=${encodeURIComponent(id)}`
}

async function loadVulnWatchConfig() {
  try {
    vulnWatch.value = await apiRequest<VulnWatchConfig>('/security/vuln/config')
  } catch {
    // ignore if unavailable
  }
}

async function saveVulnWatchConfig() {
  errorMsg.value = ''
  try {
    vulnWatch.value = await apiRequest<VulnWatchConfig>('/security/vuln/config', {
      method: 'PUT',
      body: JSON.stringify(vulnWatch.value),
    })
    notice.value = '定时 CVE 巡检与高危告警策略已保存并生效'
  } catch (err: any) {
    errorMsg.value = err?.message || '保存漏洞巡检策略失败'
  }
}

async function runVulnScan(refreshIndex = false) {
  vulnScanning.value = true
  errorMsg.value = ''
  try {
    vulnReport.value = await apiRequest<VulnScanReport>('/security/vuln/scan', {
      method: 'POST',
      body: JSON.stringify({ refresh_index: refreshIndex }),
    })
    selectedPkgs.value = vulnReport.value.packages
      .filter((p) => p.severity === 'critical' || p.severity === 'high' || p.is_security_repo)
      .map((p) => p.package)
    if (refreshIndex) {
      notice.value = vulnReport.value.summary
    }
  } catch (err: any) {
    errorMsg.value = err?.message || '执行漏洞扫描失败'
  } finally {
    vulnScanning.value = false
  }
}

function togglePkg(pkg: string) {
  if (selectedPkgs.value.includes(pkg)) {
    selectedPkgs.value = selectedPkgs.value.filter((p) => p !== pkg)
  } else {
    selectedPkgs.value.push(pkg)
  }
}

async function fixPackages(pkgs: string[], fixAllSecurity = false) {
  const tip = fixAllSecurity
    ? '确定一键升级所有高危及安全通道 (-security) 漏洞补丁吗？（自动保留现有配置文件）'
    : `确定升级修复选中的 ${pkgs.length} 个软件包（${pkgs.slice(0, 5).join(', ')}${pkgs.length > 5 ? '...' : ''}）吗？`
  if (!window.confirm(tip)) return
  vulnFixing.value = true
  errorMsg.value = ''
  fixOutput.value = '正在执行安全补丁升级，请稍候...'
  try {
    fixOutput.value = await apiRequest<string>('/security/vuln/fix', {
      method: 'POST',
      body: JSON.stringify({ packages: pkgs, fix_all_security: fixAllSecurity }),
    })
    notice.value = '安全补丁修复已完成，已自动刷新最新漏洞扫描结果'
    await runVulnScan(false)
  } catch (err: any) {
    errorMsg.value = err?.message || '漏洞修复执行失败'
  } finally {
    vulnFixing.value = false
  }
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

onMounted(() => {
  loadOverview()
  loadVulnWatchConfig()
  runVulnScan(false)
})
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
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'vuln' }" @click="activeTab = 'vuln'">
        <ShieldAlert class="h-3.5 w-3.5" /> 漏洞扫描与修复
        <span
          v-if="vulnReport && (vulnReport.critical_count + vulnReport.high_count) > 0"
          class="ml-1 rounded-full px-1.5 py-0.2 text-[10px] font-bold"
          style="background: rgba(244,63,94,0.2); color: #f43f5e"
        >
          {{ vulnReport.critical_count + vulnReport.high_count }}
        </span>
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

    <!-- Tab 2: Vulnerability Scan & One-Click Fix -->
    <div v-if="activeTab === 'vuln'" class="space-y-4 text-xs">
      <div class="zm-card p-4 flex flex-wrap items-center justify-between gap-3">
        <div class="space-y-1">
          <div class="font-semibold text-sm flex items-center gap-2">
            <span>系统漏洞扫描与安全基线体检</span>
            <span
              v-if="vulnReport"
              class="rounded-full px-2 py-0.5 text-xs font-bold"
              :style="
                vulnReport.security_score >= 85
                  ? { background: 'rgba(16,185,129,0.14)', color: '#10b981' }
                  : vulnReport.security_score >= 60
                    ? { background: 'rgba(245,158,11,0.14)', color: '#f59e0b' }
                    : { background: 'rgba(244,63,94,0.14)', color: '#f43f5e' }
              "
            >
              安全评分 {{ vulnReport.security_score }} / 100
            </span>
          </div>
          <p class="opacity-75 m-0">
            {{
              vulnReport
                ? `${vulnReport.summary}（内核: ${vulnReport.kernel_running} · 发行版: ${vulnReport.os_family}）`
                : '覆盖 Linux 内核、sudo/polkit/glibc 提权组件、OpenSSH、Nginx/OpenSSL、Node.js、Docker 及公网高危端口暴露体检'
            }}
          </p>
        </div>
        <div class="flex flex-wrap gap-2">
          <button class="zm-btn text-xs" :disabled="vulnScanning" @click="runVulnScan(true)">
            <RefreshCw class="h-3.5 w-3.5" :class="{ 'animate-spin': vulnScanning }" />
            {{ vulnScanning ? '正在刷新源并扫描...' : '刷新官方源并全面扫描' }}
          </button>
          <button
            class="zm-btn zm-btn-primary text-xs"
            :disabled="vulnFixing || !vulnReport || vulnReport.packages.length === 0"
            @click="fixPackages([], true)"
          >
            {{ vulnFixing ? '修复中...' : '一键修复全部高危/安全补丁' }}
          </button>
        </div>
      </div>

      <div
        v-if="vulnReport?.reboot_required"
        class="zm-card p-3.5 text-xs"
        style="border-color: var(--zm-warning); color: var(--zm-warning)"
      >
        系统检测到 /var/run/reboot-required：已有新内核或核心库安全补丁安装完成，请择机重启服务器以使补丁正式生效。
      </div>

      <!-- Scheduled CVE Watcher & High-Severity Alert Policy -->
      <div class="zm-card p-4 space-y-3">
        <div class="flex flex-wrap items-center justify-between gap-2">
          <div>
            <div class="font-semibold">定时 CVE 自动巡检与高危告警（默认仅严重/高危告警，中危/低危静音）</div>
            <div class="text-[11px] opacity-70 mt-0.5">
              自动刷新官方安全索引检测新 CVE，同版本去重仅告警一次；需在
              <button class="underline cursor-pointer" style="color: var(--zm-accent)" @click="router.push('/settings')">
                系统设置 → 告警通知
              </button>
              配置 Telegram 或 Webhook 通道
            </div>
          </div>
          <button class="zm-btn zm-btn-primary !py-1.5 text-xs" @click="saveVulnWatchConfig">
            <CheckCircle2 class="h-3.5 w-3.5" /> 保存巡检策略
          </button>
        </div>
        <div class="grid grid-cols-1 md:grid-cols-4 gap-3 items-center">
          <label class="flex items-center gap-2 cursor-pointer font-medium">
            <input v-model="vulnWatch.enabled" type="checkbox" />
            开启后台定时 CVE 巡检
          </label>
          <div>
            <label class="block mb-1 opacity-75">巡检周期</label>
            <select v-model.number="vulnWatch.interval_hours" class="zm-input">
              <option :value="1">每 1 小时</option>
              <option :value="3">每 3 小时</option>
              <option :value="6">每 6 小时（推荐）</option>
              <option :value="12">每 12 小时</option>
              <option :value="24">每 24 小时</option>
            </select>
          </div>
          <div>
            <label class="block mb-1 opacity-75">推送告警阈值</label>
            <select v-model="vulnWatch.alert_min_severity" class="zm-input">
              <option value="high">仅严重 + 高危告警（中危/低危不告警）</option>
              <option value="critical">仅严重漏洞告警（Critical）</option>
            </select>
          </div>
          <label class="flex items-center gap-2 cursor-pointer pt-4">
            <input v-model="vulnWatch.auto_fix_non_kernel" type="checkbox" />
            <span>自动静默修复非内核高危安全包</span>
          </label>
        </div>
        <div v-if="vulnWatch.last_scan_at > 0" class="text-[11px] opacity-65 flex flex-wrap gap-4">
          <span>上次自动巡检: {{ formatTimestamp(vulnWatch.last_scan_at) }}</span>
          <span v-if="vulnWatch.last_alert_at > 0">上次高危告警: {{ formatTimestamp(vulnWatch.last_alert_at) }}</span>
        </div>
      </div>

      <div v-if="fixOutput" class="zm-card p-3 space-y-1.5">
        <div class="flex items-center justify-between font-semibold">
          <span>补丁修复执行日志</span>
          <button class="opacity-60 hover:opacity-100 cursor-pointer" @click="fixOutput = ''">清空</button>
        </div>
        <pre
          class="p-2.5 rounded-lg text-[11px] font-mono overflow-auto max-h-56 m-0"
          style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
        >{{ fixOutput }}</pre>
      </div>

      <!-- Upgradable Security Packages -->
      <div v-if="vulnReport" class="zm-card overflow-hidden">
        <div class="px-4 py-3 border-b flex flex-wrap items-center justify-between gap-2" style="border-color: var(--zm-border)">
          <div class="font-semibold">
            待修复 / 可升级软件包 ({{ vulnReport.packages.length }})
          </div>
          <button
            v-if="selectedPkgs.length > 0"
            class="zm-btn zm-btn-primary !py-1 text-xs"
            :disabled="vulnFixing"
            @click="fixPackages(selectedPkgs, false)"
          >
            批量修复已勾选 ({{ selectedPkgs.length }})
          </button>
        </div>
        <div class="overflow-x-auto">
          <table class="w-full text-xs text-left border-collapse">
            <thead>
              <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
                <th class="py-2.5 px-3 w-8">选</th>
                <th class="py-2.5 px-2">软件包</th>
                <th class="py-2.5 px-2">风险等级</th>
                <th class="py-2.5 px-2">分类</th>
                <th class="py-2.5 px-2">当前版本 → 修复版本</th>
                <th class="py-2.5 px-2">安全说明</th>
                <th class="py-2.5 px-3 text-right">操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="vulnReport.packages.length === 0">
                <td colspan="7" class="py-8 text-center opacity-65">
                  当前未发现待升级的发行版软件包（可点击上方「刷新官方源并全面扫描」拉取最新安全索引）
                </td>
              </tr>
              <tr v-for="p in vulnReport.packages" :key="p.package" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
                <td class="py-2 px-3">
                  <input type="checkbox" :checked="selectedPkgs.includes(p.package)" @change="togglePkg(p.package)" />
                </td>
                <td class="py-2 px-2 font-mono font-semibold">
                  {{ p.package }}
                  <span
                    v-if="p.is_security_repo"
                    class="ml-1 rounded px-1 py-0.2 text-[10px]"
                    style="background: rgba(244,63,94,0.12); color: #f43f5e"
                  >
                    security
                  </span>
                </td>
                <td class="py-2 px-2">
                  <span
                    class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
                    :style="{ color: SEV_MAP[p.severity]?.color, background: SEV_MAP[p.severity]?.bg }"
                  >
                    {{ SEV_MAP[p.severity]?.label || p.severity }}
                  </span>
                </td>
                <td class="py-2 px-2 whitespace-nowrap">{{ VULN_CAT_MAP[p.category] || p.category }}</td>
                <td class="py-2 px-2 font-mono text-[11px]">
                  <span class="opacity-65">{{ p.current_version || '-' }}</span>
                  <span class="mx-1">→</span>
                  <span style="color: var(--zm-accent)">{{ p.candidate_version }}</span>
                </td>
                <td class="py-2 px-2 opacity-80 max-w-sm space-y-1">
                  <div>{{ p.advisory }}</div>
                  <div v-if="p.cves && p.cves.length > 0" class="flex flex-wrap gap-1 pt-0.5">
                    <a
                      v-for="cve in p.cves"
                      :key="cve"
                      :href="cveUrl(cve)"
                      target="_blank"
                      rel="noopener noreferrer"
                      class="rounded px-1.5 py-0.5 text-[10px] font-mono font-semibold hover:underline"
                      style="background: rgba(244,63,94,0.14); color: #f43f5e"
                    >
                      {{ cve }}
                    </a>
                  </div>
                </td>
                <td class="py-2 px-3 text-right whitespace-nowrap">
                  <button
                    class="zm-btn !py-1 !px-2 text-[11px]"
                    :disabled="vulnFixing"
                    @click="fixPackages([p.package], false)"
                  >
                    一键修复
                  </button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>

      <!-- Security Baseline Checks -->
      <div v-if="vulnReport" class="zm-card p-4 space-y-3">
        <div class="font-semibold">核心安全基线体检（内核 / 提权账户 / OpenSSH / Node.js / Nginx / 高危端口暴露）</div>
        <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
          <div
            v-for="b in vulnReport.baselines"
            :key="b.id"
            class="rounded-lg p-3 space-y-1.5"
            style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
          >
            <div class="flex items-center justify-between gap-2">
              <span class="font-semibold">{{ b.title }}</span>
              <span
                class="rounded px-1.5 py-0.5 text-[10px] font-semibold shrink-0"
                :style="{ color: SEV_MAP[b.severity]?.color, background: SEV_MAP[b.severity]?.bg }"
              >
                {{ SEV_MAP[b.severity]?.label || b.severity }}
              </span>
            </div>
            <div class="opacity-80 leading-relaxed">{{ b.detail }}</div>
            <div v-if="b.cves && b.cves.length > 0" class="flex flex-wrap gap-1">
              <a
                v-for="cve in b.cves"
                :key="cve"
                :href="cveUrl(cve)"
                target="_blank"
                rel="noopener noreferrer"
                class="rounded px-1.5 py-0.5 text-[10px] font-mono font-semibold hover:underline"
                style="background: rgba(244,63,94,0.14); color: #f43f5e"
              >
                {{ cve }}
              </a>
            </div>
            <div class="text-[11px] opacity-65">建议：{{ b.remediation }}</div>
          </div>
        </div>
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
