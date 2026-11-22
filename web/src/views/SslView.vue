<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Copy, Download, Lock, Plus, RefreshCw, ShieldCheck, Trash2, Upload, X } from 'lucide-vue-next'
import { apiRequest, formatTimestamp } from '@/api/client'

interface SslCertificate {
  id: string
  name: string
  domains: string[]
  issuer: string
  provider: string
  challenge_type: string
  dns_provider?: string | null
  auto_renew: boolean
  renew_hook: string
  not_before: number
  not_after: number
  cert_path: string
  key_path: string
  last_renew_status: string
}

const certs = ref<SslCertificate[]>([])
const notice = ref('')
const errorMsg = ref('')

const modalMode = ref<'none' | 'acme' | 'import' | 'self'>('none')
const acmeForm = ref({
  name: '',
  domainsText: '',
  provider: 'letsencrypt',
  challenge_type: 'dns01',
  dns_provider: 'cloudflare',
  dns_token: '',
  auto_renew: true,
  renew_hook: 'nginx -s reload',
})
const importForm = ref({
  name: '',
  cert_pem: '',
  key_pem: '',
  renew_hook: '',
})
const selfForm = ref({
  name: '',
  domainsText: 'localhost\n*.local.test',
  valid_days: 365,
})

const exportModal = ref<{ open: boolean; cert_pem: string; key_pem: string; name: string }>({
  open: false,
  cert_pem: '',
  key_pem: '',
  name: '',
})

function showMsg(msg: string) {
  notice.value = msg
  errorMsg.value = ''
  setTimeout(() => {
    if (notice.value === msg) notice.value = ''
  }, 3500)
}

function remainDays(notAfter: number): number {
  if (!notAfter) return 0
  return Math.floor((notAfter - Date.now() / 1000) / 86400)
}

async function loadCerts() {
  try {
    certs.value = await apiRequest<SslCertificate[]>('/ssl/certs')
  } catch (err: any) {
    errorMsg.value = err?.message || '加载证书列表失败'
  }
}

async function submitAcme() {
  const domains = acmeForm.value.domainsText
    .split(/[\n,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean)
  try {
    await apiRequest('/ssl/certs/acme', {
      method: 'POST',
      body: JSON.stringify({
        name: acmeForm.value.name,
        domains,
        provider: acmeForm.value.provider,
        challenge_type: acmeForm.value.challenge_type,
        dns_provider: acmeForm.value.challenge_type === 'dns01' ? acmeForm.value.dns_provider : null,
        dns_credentials_json: acmeForm.value.dns_token
          ? JSON.stringify({ token: acmeForm.value.dns_token })
          : null,
        auto_renew: acmeForm.value.auto_renew,
        renew_hook: acmeForm.value.renew_hook,
      }),
    })
    modalMode.value = 'none'
    showMsg('ACME 证书配置已保存')
    await loadCerts()
  } catch (err: any) {
    errorMsg.value = err?.message || '保存 ACME 证书失败'
  }
}

async function submitImport() {
  try {
    await apiRequest('/ssl/certs/import', {
      method: 'POST',
      body: JSON.stringify(importForm.value),
    })
    modalMode.value = 'none'
    importForm.value = { name: '', cert_pem: '', key_pem: '', renew_hook: '' }
    showMsg('证书解析并导入成功')
    await loadCerts()
  } catch (err: any) {
    errorMsg.value = err?.message || '导入证书失败'
  }
}

async function submitSelfSigned() {
  const domains = selfForm.value.domainsText
    .split(/[\n,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean)
  try {
    await apiRequest('/ssl/certs/self-signed', {
      method: 'POST',
      body: JSON.stringify({
        name: selfForm.value.name,
        domains,
        valid_days: Number(selfForm.value.valid_days) || 365,
      }),
    })
    modalMode.value = 'none'
    showMsg('自签名 SSL 证书已生成')
    await loadCerts()
  } catch (err: any) {
    errorMsg.value = err?.message || '生成失败'
  }
}

async function openExport(c: SslCertificate) {
  try {
    const res = await apiRequest<{ cert_pem: string; key_pem: string }>(`/ssl/certs/${c.id}/export`)
    exportModal.value = { open: true, cert_pem: res.cert_pem, key_pem: res.key_pem, name: c.name }
  } catch (err: any) {
    errorMsg.value = err?.message || '导出失败'
  }
}

async function deleteCert(c: SslCertificate) {
  if (!window.confirm(`确定删除证书「${c.name}」及本地私钥文件吗？`)) return
  await apiRequest(`/ssl/certs/${c.id}`, { method: 'DELETE' })
  showMsg('已删除证书')
  await loadCerts()
}

function copyText(txt: string) {
  navigator.clipboard.writeText(txt).then(() => showMsg('已复制到剪贴板'))
}

onMounted(loadCerts)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <Lock class="h-5 w-5" style="color: var(--zm-accent)" />
          SSL 证书中心
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          完全独立于建站，统一管理通配符与任意域名证书（ACME / 手动导入 / 纯 Rust 自签名）
        </p>
      </div>
      <div class="flex flex-wrap gap-2">
        <button class="zm-btn text-xs" @click="loadCerts">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
        <button class="zm-btn text-xs" @click="modalMode = 'self'">
          <ShieldCheck class="h-3.5 w-3.5" /> 自签名证书
        </button>
        <button class="zm-btn text-xs" @click="modalMode = 'import'">
          <Upload class="h-3.5 w-3.5" /> 导入 PEM
        </button>
        <button class="zm-btn zm-btn-primary text-xs" @click="modalMode = 'acme'">
          <Plus class="h-3.5 w-3.5" /> ACME 申请
        </button>
      </div>
    </div>

    <div v-if="notice" class="zm-card px-3.5 py-2 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card px-3.5 py-2 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <div class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">名称 / 域名</th>
              <th class="py-2.5 px-2">颁发机构 / 来源</th>
              <th class="py-2.5 px-2">有效期</th>
              <th class="py-2.5 px-2">证书路径</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="certs.length === 0">
              <td colspan="5" class="py-12 text-center opacity-60">
                暂无证书，可导入已有证书、签发自签名证书或添加 ACME 自动续期配置
              </td>
            </tr>
            <tr v-for="c in certs" :key="c.id" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
              <td class="py-2.5 px-3">
                <div class="font-semibold">{{ c.name }}</div>
                <div class="font-mono text-[11px] opacity-80">{{ c.domains.join(', ') }}</div>
              </td>
              <td class="py-2.5 px-2">
                <div>{{ c.issuer }}</div>
                <div class="text-[11px] opacity-60">{{ c.provider }} {{ c.auto_renew ? '· 自动续期' : '' }}</div>
              </td>
              <td class="py-2.5 px-2">
                <span
                  v-if="remainDays(c.not_after) > 30"
                  class="text-emerald-400 font-medium"
                >
                  剩余 {{ remainDays(c.not_after) }} 天
                </span>
                <span
                  v-else-if="remainDays(c.not_after) > 0"
                  class="text-amber-400 font-medium"
                >
                  即将过期（{{ remainDays(c.not_after) }} 天）
                </span>
                <span v-else class="text-rose-400">待签发 / 已过期</span>
                <div class="text-[10px] opacity-60">止于 {{ formatTimestamp(c.not_after) }}</div>
              </td>
              <td class="py-2.5 px-2 font-mono text-[11px] max-w-xs truncate">
                <button class="hover:underline cursor-pointer" title="点击复制证书路径" @click="copyText(c.cert_path)">
                  {{ c.cert_path }}
                </button>
              </td>
              <td class="py-2.5 px-3 text-right space-x-1 whitespace-nowrap">
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="openExport(c)">
                  <Download class="h-3 w-3" /> 查看/导出
                </button>
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="deleteCert(c)">
                  <Trash2 class="h-3 w-3" />
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- ACME Modal -->
    <div v-if="modalMode === 'acme'" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4">
      <div class="zm-card w-full max-w-lg p-5 space-y-3 text-xs">
        <div class="flex items-center justify-between font-semibold text-sm">
          <span>ACME 自动证书申请配置</span>
          <button @click="modalMode = 'none'"><X class="h-4 w-4" /></button>
        </div>
        <input v-model="acmeForm.name" class="zm-input" placeholder="备注名称（可选，默认取首域名）" />
        <textarea v-model="acmeForm.domainsText" rows="2" class="zm-input font-mono" placeholder="域名列表，支持通配符，如：&#10;example.com&#10;*.example.com" />
        <div class="grid grid-cols-2 gap-2">
          <select v-model="acmeForm.provider" class="zm-input">
            <option value="letsencrypt">Let's Encrypt</option>
            <option value="zerossl">ZeroSSL</option>
          </select>
          <select v-model="acmeForm.challenge_type" class="zm-input">
            <option value="dns01">DNS-01（支持通配符）</option>
            <option value="http01">HTTP-01（80 端口验证）</option>
          </select>
        </div>
        <div v-if="acmeForm.challenge_type === 'dns01'" class="grid grid-cols-2 gap-2">
          <select v-model="acmeForm.dns_provider" class="zm-input">
            <option value="cloudflare">Cloudflare API Token</option>
            <option value="aliyun">阿里云 DNS</option>
            <option value="dnspod">腾讯云 DNSPod</option>
          </select>
          <input v-model="acmeForm.dns_token" type="password" class="zm-input" placeholder="DNS API Token / Secret" />
        </div>
        <input v-model="acmeForm.renew_hook" class="zm-input font-mono" placeholder="续期后 Hook 命令（如 nginx -s reload）" />
        <label class="flex items-center gap-2 cursor-pointer">
          <input v-model="acmeForm.auto_renew" type="checkbox" /> 到期前 30 天自动续期
        </label>
        <div class="flex justify-end gap-2 pt-2">
          <button class="zm-btn" @click="modalMode = 'none'">取消</button>
          <button class="zm-btn zm-btn-primary" @click="submitAcme">保存配置</button>
        </div>
      </div>
    </div>

    <!-- Import PEM Modal -->
    <div v-if="modalMode === 'import'" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4">
      <div class="zm-card w-full max-w-xl p-5 space-y-3 text-xs">
        <div class="flex items-center justify-between font-semibold text-sm">
          <span>手动导入 PEM 证书与私钥</span>
          <button @click="modalMode = 'none'"><X class="h-4 w-4" /></button>
        </div>
        <input v-model="importForm.name" class="zm-input" placeholder="证书备注名（留空自动从 X.509 域名提取）" />
        <textarea v-model="importForm.cert_pem" rows="5" class="zm-input font-mono" placeholder="-----BEGIN CERTIFICATE-----" />
        <textarea v-model="importForm.key_pem" rows="5" class="zm-input font-mono" placeholder="-----BEGIN PRIVATE KEY-----" />
        <div class="flex justify-end gap-2">
          <button class="zm-btn" @click="modalMode = 'none'">取消</button>
          <button class="zm-btn zm-btn-primary" @click="submitImport">解析并导入</button>
        </div>
      </div>
    </div>

    <!-- Self-Signed Modal -->
    <div v-if="modalMode === 'self'" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4">
      <div class="zm-card w-full max-w-md p-5 space-y-3 text-xs">
        <div class="flex items-center justify-between font-semibold text-sm">
          <span>生成纯 Rust 自签名证书</span>
          <button @click="modalMode = 'none'"><X class="h-4 w-4" /></button>
        </div>
        <input v-model="selfForm.name" class="zm-input" placeholder="证书名称（可选）" />
        <textarea v-model="selfForm.domainsText" rows="3" class="zm-input font-mono" placeholder="每行一个域名" />
        <div>
          <label class="block mb-1 opacity-75">有效天数</label>
          <input v-model.number="selfForm.valid_days" type="number" class="zm-input" />
        </div>
        <div class="flex justify-end gap-2">
          <button class="zm-btn" @click="modalMode = 'none'">取消</button>
          <button class="zm-btn zm-btn-primary" @click="submitSelfSigned">立即签发</button>
        </div>
      </div>
    </div>

    <!-- Export Modal -->
    <div v-if="exportModal.open" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4">
      <div class="zm-card w-full max-w-2xl p-5 space-y-3 text-xs">
        <div class="flex items-center justify-between font-semibold text-sm">
          <span>查看 / 导出证书 · {{ exportModal.name }}</span>
          <button @click="exportModal.open = false"><X class="h-4 w-4" /></button>
        </div>
        <div class="flex items-center justify-between">
          <span class="font-medium">证书链 (fullchain.pem)</span>
          <button class="zm-btn !py-1 text-[11px]" @click="copyText(exportModal.cert_pem)">
            <Copy class="h-3 w-3" /> 复制证书
          </button>
        </div>
        <textarea readonly :value="exportModal.cert_pem" rows="6" class="zm-input font-mono text-[11px]" />
        <div class="flex items-center justify-between">
          <span class="font-medium">私钥 (privkey.pem)</span>
          <button class="zm-btn !py-1 text-[11px]" @click="copyText(exportModal.key_pem)">
            <Copy class="h-3 w-3" /> 复制私钥
          </button>
        </div>
        <textarea readonly :value="exportModal.key_pem" rows="5" class="zm-input font-mono text-[11px]" />
      </div>
    </div>
  </div>
</template>
