<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Bell, Download, KeyRound, Lock, Palette, Power, RotateCw, ScrollText, Shield, ShieldCheck } from 'lucide-vue-next'
import { apiRequest, formatTimestamp } from '@/api/client'
import { useAuthStore } from '@/stores/auth'
import { ACCENT_PRESETS, THEME_PRESETS, useThemeStore } from '@/stores/theme'

interface AuditLogItem {
  id: number
  timestamp: number
  actor: string
  ip: string
  action: string
  target: string
  detail: string
}

const auth = useAuthStore()
const theme = useThemeStore()

const securityEntry = ref(auth.profile?.security_entry?.replace(/^\/+/, '') || '')
const currentPassword = ref('')
const newPassword = ref('')
const totpSetupData = ref<{ secret: string; otpauth_url: string } | null>(null)
const totpCode = ref('')
const auditLogs = ref<AuditLogItem[]>([])
const feedback = ref('')
const feedbackErr = ref('')

// Self-update & Notifications (Stage 4)
const manifestUrl = ref('')
const updateManifest = ref<{ version: string; changelog: string; download_url: string; sha256: string } | null>(null)
const updating = ref(false)
const notifyCfg = ref({
  enabled: false,
  webhook_url: '',
  telegram_bot_token: '',
  telegram_chat_id: '',
  notify_on_ban: true,
  notify_on_node_offline: true,
})

function setMsg(ok: string, err = '') {
  feedback.value = ok
  feedbackErr.value = err
}

async function saveSecurityEntry() {
  setMsg('')
  try {
    const newPath = await apiRequest<string>('/settings/security', {
      method: 'PUT',
      body: JSON.stringify({ security_entry: securityEntry.value }),
    })
    setMsg(`安全入口已实时生效：${window.location.origin}${newPath}（旧入口已立即失效，正在跳转...）`)
    setTimeout(() => {
      window.location.href = `${window.location.origin}${newPath}/#/settings`
    }, 1500)
  } catch (err: any) {
    setMsg('', err?.message || '修改安全入口失败')
  }
}

async function savePassword() {
  setMsg('')
  try {
    await apiRequest('/auth/password', {
      method: 'PUT',
      body: JSON.stringify({
        current_password: currentPassword.value,
        new_password: newPassword.value,
      }),
    })
    currentPassword.value = ''
    newPassword.value = ''
    setMsg('管理员密码修改成功，请重新登录。')
    setTimeout(() => {
      auth.logout().then(() => window.location.reload())
    }, 1200)
  } catch (err: any) {
    setMsg('', err?.message || '修改密码失败')
  }
}

async function startTotpSetup() {
  setMsg('')
  try {
    totpSetupData.value = await apiRequest<{ secret: string; otpauth_url: string }>(
      '/auth/totp/setup',
      { method: 'POST' },
    )
  } catch (err: any) {
    setMsg('', err?.message || '生成 2FA 密钥失败')
  }
}

async function enableTotp() {
  setMsg('')
  try {
    await apiRequest('/auth/totp/enable', {
      method: 'POST',
      body: JSON.stringify({ code: totpCode.value }),
    })
    totpSetupData.value = null
    totpCode.value = ''
    await auth.fetchMe()
    setMsg('TOTP 双因素认证已开启')
  } catch (err: any) {
    setMsg('', err?.message || '开启 2FA 失败')
  }
}

async function disableTotp() {
  const code = window.prompt('请输入当前 6 位 2FA 动态码以关闭双因素认证：')
  if (!code) return
  setMsg('')
  try {
    await apiRequest('/auth/totp/disable', {
      method: 'POST',
      body: JSON.stringify({ code }),
    })
    await auth.fetchMe()
    setMsg('TOTP 双因素认证已关闭')
  } catch (err: any) {
    setMsg('', err?.message || '关闭 2FA 失败')
  }
}

async function loadAuditLogs() {
  try {
    auditLogs.value = await apiRequest<AuditLogItem[]>('/system/audit?limit=60')
  } catch {
    auditLogs.value = []
  }
}

async function loadNotifyConfig() {
  try {
    notifyCfg.value = await apiRequest<typeof notifyCfg.value>('/notify/config')
  } catch {}
}

async function saveNotify() {
  setMsg('')
  try {
    await apiRequest('/notify/config', { method: 'PUT', body: JSON.stringify(notifyCfg.value) })
    setMsg('告警通知配置已保存（默认零外联，仅在开启后推送）')
  } catch (err: any) {
    setMsg('', err?.message || '保存告警配置失败')
  }
}

async function testNotify() {
  setMsg('')
  try {
    const res = await apiRequest<string>('/notify/test', { method: 'POST' })
    setMsg(`测试告警发送成功：${res}`)
  } catch (err: any) {
    setMsg('', err?.message || '测试推送失败')
  }
}

async function checkUpdate() {
  if (!manifestUrl.value.trim()) return
  setMsg('')
  try {
    updateManifest.value = await apiRequest('/update/check', {
      method: 'POST',
      body: JSON.stringify({ manifest_url: manifestUrl.value.trim() }),
    })
    setMsg(`检测到发布版本 v${updateManifest.value?.version}`)
  } catch (err: any) {
    setMsg('', err?.message || '检查更新失败')
  }
}

async function applySelfUpdate() {
  if (!updateManifest.value) return
  if (!window.confirm(`确定下载校验 SHA256 并升级至 v${updateManifest.value.version} 吗？（旧版将自动备份为 .bak）`)) return
  updating.value = true
  setMsg('')
  try {
    const res = await apiRequest<string>('/update/apply', {
      method: 'POST',
      body: JSON.stringify({
        download_url: updateManifest.value.download_url,
        sha256: updateManifest.value.sha256,
        target_version: updateManifest.value.version,
      }),
    })
    setMsg(res)
  } catch (err: any) {
    setMsg('', err?.message || '执行自更新失败')
  } finally {
    updating.value = false
  }
}

async function triggerPower(target: 'panel' | 'server') {
  const tip =
    target === 'panel'
      ? '确定重启 zmpanel 面板服务吗？（约 2~3 秒后恢复）'
      : '危险操作：确定立即重启整台服务器系统吗？（30~60 秒中断）'
  if (!window.confirm(tip)) return
  setMsg('')
  try {
    const msg = await apiRequest<string>('/system/power', {
      method: 'POST',
      body: JSON.stringify({ target }),
    })
    setMsg(msg)
  } catch (err: any) {
    setMsg('', err?.message || '触发重启失败')
  }
}

onMounted(() => {
  securityEntry.value = auth.profile?.security_entry?.replace(/^\/+/, '') || ''
  loadAuditLogs()
  loadNotifyConfig()
})
</script>

<template>
  <div class="space-y-5 max-w-5xl">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold">面板设置与安全</h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          完全离线本地配置 · 零遥测零上传 · 多套高颜值主题自由切换
        </p>
      </div>
      <div class="flex items-center gap-2">
        <button class="zm-btn text-xs" @click="triggerPower('panel')">
          <RotateCw class="h-3.5 w-3.5" style="color: var(--zm-accent)" /> 重启面板
        </button>
        <button class="zm-btn zm-btn-danger text-xs" @click="triggerPower('server')">
          <Power class="h-3.5 w-3.5" /> 重启服务器
        </button>
      </div>
    </div>

    <div v-if="feedback" class="zm-card p-3 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ feedback }}
    </div>
    <div v-if="feedbackErr" class="zm-card p-3 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ feedbackErr }}
    </div>

    <!-- Themes -->
    <div class="zm-card p-5 space-y-4">
      <div class="flex items-center gap-2 font-semibold text-sm">
        <Palette class="h-4 w-4" style="color: var(--zm-accent)" />
        外观与多主题皮肤
      </div>
      <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
        <button
          v-for="t in THEME_PRESETS"
          :key="t.id"
          class="flex items-center gap-3 rounded-xl p-3 text-left border transition cursor-pointer"
          :style="{
            borderColor: theme.themeId === t.id ? 'var(--zm-accent)' : 'var(--zm-border)',
            background: theme.themeId === t.id ? 'var(--zm-accent-soft)' : 'var(--zm-surface-elevated)',
          }"
          @click="theme.setTheme(t.id)"
        >
          <div
            class="h-11 w-14 shrink-0 rounded-lg overflow-hidden border border-white/15 flex"
            :style="{ background: t.previewBg }"
          >
            <div class="w-4 h-full" :style="{ background: t.previewSidebar }" />
            <div class="flex-1 p-1.5 flex flex-col gap-1">
              <div class="h-2 rounded-sm" :style="{ background: t.previewSurface }" />
              <div class="flex-1 rounded-sm" :style="{ background: t.previewSurface }" />
            </div>
          </div>
          <div class="min-w-0">
            <div class="text-xs font-semibold">{{ t.name }}</div>
            <div class="text-[11px] truncate mt-0.5" style="color: var(--zm-text-muted)">
              {{ t.desc }}
            </div>
          </div>
        </button>
      </div>

      <div class="pt-1">
        <div class="text-xs font-medium mb-2" style="color: var(--zm-text-muted)">主强调色</div>
        <div class="flex flex-wrap gap-2.5">
          <button
            v-for="a in ACCENT_PRESETS"
            :key="a.id"
            class="flex items-center gap-2 rounded-lg px-3 py-1.5 text-xs border cursor-pointer"
            :style="{
              borderColor: theme.accentId === a.id ? a.color : 'var(--zm-border)',
              background: theme.accentId === a.id ? a.soft : 'var(--zm-surface-elevated)',
            }"
            @click="theme.setAccent(a.id)"
          >
            <span class="h-3.5 w-3.5 rounded-full" :style="{ background: a.color }" />
            <span>{{ a.name }}</span>
          </button>
        </div>
      </div>
    </div>

    <!-- Security Settings -->
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <div class="zm-card p-5 space-y-3">
        <div class="flex items-center gap-2 font-semibold text-sm">
          <Shield class="h-4 w-4" style="color: var(--zm-accent)" />
          随机安全入口路径
        </div>
        <p class="text-xs" style="color: var(--zm-text-muted)">
          访问非安全入口路径将直接返回伪装 404，避免公网扫描器发现面板。修改后即时生效，无需重启。
        </p>
        <div class="flex items-center gap-2">
          <span class="text-xs font-mono opacity-60">/</span>
          <input v-model="securityEntry" class="zm-input font-mono text-xs" placeholder="zm_myentry" />
          <button class="zm-btn zm-btn-primary shrink-0 text-xs" @click="saveSecurityEntry">
            保存生效
          </button>
        </div>
      </div>

      <div class="zm-card p-5 space-y-3">
        <div class="flex items-center gap-2 font-semibold text-sm">
          <Lock class="h-4 w-4" style="color: var(--zm-accent)" />
          修改管理员密码
        </div>
        <input v-model="currentPassword" type="password" class="zm-input text-xs" placeholder="当前密码" />
        <input v-model="newPassword" type="password" class="zm-input text-xs" placeholder="新密码（至少 8 位）" />
        <button class="zm-btn zm-btn-primary text-xs" @click="savePassword">更新密码</button>
      </div>
    </div>

    <!-- 2FA -->
    <div class="zm-card p-5 space-y-3">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2 font-semibold text-sm">
          <ShieldCheck class="h-4 w-4" style="color: var(--zm-accent)" />
          双因素认证（TOTP 2FA）
        </div>
        <span
          class="rounded-full px-2.5 py-0.5 text-xs font-semibold"
          :style="
            auth.profile?.totp_enabled
              ? { background: 'var(--zm-accent-soft)', color: 'var(--zm-accent)' }
              : { background: 'var(--zm-surface-elevated)', color: 'var(--zm-text-muted)' }
          "
        >
          {{ auth.profile?.totp_enabled ? '已开启' : '未开启' }}
        </span>
      </div>

      <div v-if="!auth.profile?.totp_enabled && !totpSetupData">
        <button class="zm-btn zm-btn-primary text-xs" @click="startTotpSetup">
          <KeyRound class="h-3.5 w-3.5" /> 生成 2FA 密钥
        </button>
      </div>

      <div v-if="totpSetupData" class="space-y-2.5 rounded-lg p-3.5" style="background: var(--zm-surface-elevated)">
        <div class="text-xs">
          请在 Authenticator（Google Authenticator / Bitwarden / 1Password）中手动添加以下密钥：
        </div>
        <div class="font-mono text-sm font-bold tracking-wider select-all" style="color: var(--zm-accent)">
          {{ totpSetupData.secret }}
        </div>
        <div class="text-[11px] font-mono break-all opacity-70">
          {{ totpSetupData.otpauth_url }}
        </div>
        <div class="flex items-center gap-2 pt-1">
          <input v-model="totpCode" maxlength="6" class="zm-input !w-40 font-mono text-xs" placeholder="输入 6 位验证码" />
          <button class="zm-btn zm-btn-primary text-xs" @click="enableTotp">验证并开启</button>
        </div>
      </div>

      <div v-if="auth.profile?.totp_enabled">
        <button class="zm-btn zm-btn-danger text-xs" @click="disableTotp">关闭 2FA</button>
      </div>
    </div>

    <!-- Self Update & Notifications -->
    <div class="grid grid-cols-1 lg:grid-cols-2 gap-4 text-xs">
      <div class="zm-card p-5 space-y-3">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2 font-semibold text-sm">
            <Download class="h-4 w-4" style="color: var(--zm-accent)" />
            面板自主检查更新（零后台外联）
          </div>
          <span class="font-mono opacity-70">当前 v{{ auth.profile?.version || '0.1.0' }}</span>
        </div>
        <p style="color: var(--zm-text-muted)">
          仅在手动点击检查时拉取你自托管或信任的 release-manifest.json，更新前自动执行 SHA256 校验 + 新二进制 --version 自检 + 旧版 .bak 备份。
        </p>
        <div class="flex gap-2">
          <input v-model="manifestUrl" class="zm-input font-mono" placeholder="https://your-mirror/zmpanel/release-manifest.json" />
          <button class="zm-btn shrink-0" @click="checkUpdate">检查更新</button>
        </div>
        <div v-if="updateManifest" class="rounded-lg p-3 space-y-2" style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)">
          <div class="font-semibold">发现新版本：v{{ updateManifest.version }}</div>
          <div class="opacity-80 whitespace-pre-wrap">{{ updateManifest.changelog }}</div>
          <div class="font-mono text-[10px] opacity-65 break-all">SHA256: {{ updateManifest.sha256 }}</div>
          <button class="zm-btn zm-btn-primary text-xs" :disabled="updating" @click="applySelfUpdate">
            {{ updating ? '校验替换中...' : '立即安全升级' }}
          </button>
        </div>
      </div>

      <div class="zm-card p-5 space-y-3">
        <div class="flex items-center gap-2 font-semibold text-sm">
          <Bell class="h-4 w-4" style="color: var(--zm-accent)" />
          告警通知（Webhook / Telegram）
        </div>
        <label class="flex items-center gap-2 cursor-pointer">
          <input v-model="notifyCfg.enabled" type="checkbox" /> 启用告警推送（默认关闭，保持零外联）
        </label>
        <input v-model="notifyCfg.webhook_url" class="zm-input font-mono" placeholder="Webhook URL (可选)" />
        <div class="grid grid-cols-2 gap-2">
          <input v-model="notifyCfg.telegram_bot_token" type="password" class="zm-input font-mono" placeholder="Telegram Bot Token" />
          <input v-model="notifyCfg.telegram_chat_id" class="zm-input font-mono" placeholder="Telegram Chat ID" />
        </div>
        <div class="flex gap-2">
          <button class="zm-btn zm-btn-primary text-xs" @click="saveNotify">保存配置</button>
          <button class="zm-btn text-xs" @click="testNotify">发送测试通知</button>
        </div>
      </div>
    </div>

    <!-- Audit Logs -->
    <div class="zm-card p-5 space-y-3">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2 font-semibold text-sm">
          <ScrollText class="h-4 w-4" style="color: var(--zm-accent)" />
          操作审计日志（最近 60 条）
        </div>
        <button class="zm-btn !py-1 text-xs" @click="loadAuditLogs">刷新</button>
      </div>
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); color: var(--zm-text-muted)">
              <th class="py-2 px-2">时间</th>
              <th class="py-2 px-2">操作人</th>
              <th class="py-2 px-2">来源 IP</th>
              <th class="py-2 px-2">动作</th>
              <th class="py-2 px-2">对象</th>
              <th class="py-2 px-2">详情</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="log in auditLogs"
              :key="log.id"
              class="border-b"
              style="border-color: var(--zm-border)"
            >
              <td class="py-1.5 px-2 whitespace-nowrap opacity-75">{{ formatTimestamp(log.timestamp) }}</td>
              <td class="py-1.5 px-2 font-medium">{{ log.actor }}</td>
              <td class="py-1.5 px-2 font-mono">{{ log.ip }}</td>
              <td class="py-1.5 px-2 font-mono" style="color: var(--zm-accent)">{{ log.action }}</td>
              <td class="py-1.5 px-2 font-mono max-w-[180px] truncate">{{ log.target }}</td>
              <td class="py-1.5 px-2">{{ log.detail }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>
