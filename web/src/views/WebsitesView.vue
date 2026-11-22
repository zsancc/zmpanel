<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  FileCode2,
  FolderOpen,
  Globe,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  ScrollText,
  Square,
  Terminal,
  Trash2,
  X,
} from 'lucide-vue-next'
import CodeEditor from '@/components/CodeEditor.vue'
import { apiRequest } from '@/api/client'
import { useThemeStore } from '@/stores/theme'

type WebsiteKind = 'rust' | 'go' | 'python' | 'proxy' | 'static'

interface WebsiteWithRuntime {
  id: string
  name: string
  kind: WebsiteKind
  domains: string[]
  listen_port: number
  root_dir: string
  start_cmd: string
  env_vars: string
  python_venv?: string | null
  proxy_target?: string | null
  websocket_enabled: boolean
  spa_fallback: boolean
  ssl_cert_id?: string | null
  force_https: boolean
  auto_restart: boolean
  remark: string
  runtime_status: string
  pid?: number | null
  restart_count: number
  nginx_conf_path: string
}

interface SslOption {
  id: string
  name: string
  domains: string[]
}

interface VenvOption {
  id: string
  name: string
  path: string
  python_version: string
}

const KIND_TABS: { id: 'all' | WebsiteKind; label: string }[] = [
  { id: 'all', label: '全部项目' },
  { id: 'rust', label: 'Rust 项目' },
  { id: 'go', label: 'Go 项目' },
  { id: 'python', label: 'Python 项目' },
  { id: 'proxy', label: '反向代理' },
  { id: 'static', label: 'HTML 纯静态' },
]

const KIND_LABELS: Record<WebsiteKind, string> = {
  rust: 'Rust',
  go: 'Go',
  python: 'Python',
  proxy: '反向代理',
  static: 'HTML 静态',
}

const router = useRouter()
const theme = useThemeStore()

const activeTab = ref<'all' | WebsiteKind>('all')
const sites = ref<WebsiteWithRuntime[]>([])
const sslCerts = ref<SslOption[]>([])
const pythonVenvs = ref<VenvOption[]>([])
const notice = ref('')
const errorMsg = ref('')

// Create / Edit modal
const modalOpen = ref(false)
const form = ref({
  id: '' as string | undefined,
  name: '',
  kind: 'rust' as WebsiteKind,
  domainsText: '',
  listen_port: 8080,
  root_dir: '/opt/apps/demo',
  start_cmd: './app',
  env_vars: '',
  python_venv: '',
  proxy_target: 'http://127.0.0.1:8080',
  websocket_enabled: true,
  spa_fallback: false,
  ssl_cert_id: '',
  force_https: false,
  auto_restart: true,
  remark: '',
})

// Nginx editor modal
const nginxModalOpen = ref(false)
const nginxSiteId = ref('')
const nginxSiteName = ref('')
const nginxContent = ref('')

// Logs modal
const logsModalOpen = ref(false)
const logsTitle = ref('')
const logsContent = ref('')

const filteredSites = computed(() =>
  activeTab.value === 'all' ? sites.value : sites.value.filter((s) => s.kind === activeTab.value),
)

function showMsg(msg: string) {
  notice.value = msg
  errorMsg.value = ''
  setTimeout(() => {
    if (notice.value === msg) notice.value = ''
  }, 3500)
}

async function loadAll() {
  try {
    const [s, certs, uv] = await Promise.all([
      apiRequest<WebsiteWithRuntime[]>('/websites'),
      apiRequest<SslOption[]>('/ssl/certs').catch(() => []),
      apiRequest<{ venvs: VenvOption[] }>('/store/uv/overview').catch(() => ({ venvs: [] })),
    ])
    sites.value = s
    sslCerts.value = certs
    pythonVenvs.value = uv.venvs || []
  } catch (err: any) {
    errorMsg.value = err?.message || '加载网站列表失败'
  }
}

function openCreate(kind?: WebsiteKind) {
  const k = kind || (activeTab.value === 'all' ? 'rust' : activeTab.value)
  form.value = {
    id: undefined,
    name: '',
    kind: k,
    domainsText: '',
    listen_port: k === 'python' ? 8000 : 8080,
    root_dir: `/opt/apps/my-${k}-app`,
    start_cmd:
      k === 'rust'
        ? './target/release/app'
        : k === 'go'
          ? './server'
          : k === 'python'
            ? 'uvicorn main:app --host 127.0.0.1 --port 8000'
            : '',
    env_vars: '',
    python_venv: '',
    proxy_target: 'http://127.0.0.1:8080',
    websocket_enabled: true,
    spa_fallback: k === 'static',
    ssl_cert_id: '',
    force_https: false,
    auto_restart: true,
    remark: '',
  }
  modalOpen.value = true
}

function openEdit(site: WebsiteWithRuntime) {
  form.value = {
    id: site.id,
    name: site.name,
    kind: site.kind,
    domainsText: site.domains.join('\n'),
    listen_port: site.listen_port,
    root_dir: site.root_dir,
    start_cmd: site.start_cmd,
    env_vars: site.env_vars,
    python_venv: site.python_venv || '',
    proxy_target: site.proxy_target || 'http://127.0.0.1:8080',
    websocket_enabled: site.websocket_enabled,
    spa_fallback: site.spa_fallback,
    ssl_cert_id: site.ssl_cert_id || '',
    force_https: site.force_https,
    auto_restart: site.auto_restart,
    remark: site.remark,
  }
  modalOpen.value = true
}

async function saveSite() {
  errorMsg.value = ''
  const domains = form.value.domainsText
    .split(/[\n,\s]+/)
    .map((d) => d.trim())
    .filter(Boolean)
  try {
    await apiRequest('/websites', {
      method: 'POST',
      body: JSON.stringify({
        id: form.value.id,
        name: form.value.name,
        kind: form.value.kind,
        domains,
        listen_port: Number(form.value.listen_port) || 8080,
        root_dir: form.value.root_dir,
        start_cmd: form.value.start_cmd,
        env_vars: form.value.env_vars,
        python_venv: form.value.python_venv || null,
        proxy_target: form.value.proxy_target || null,
        websocket_enabled: form.value.websocket_enabled,
        spa_fallback: form.value.spa_fallback,
        ssl_cert_id: form.value.ssl_cert_id || null,
        force_https: form.value.force_https,
        auto_restart: form.value.auto_restart,
        remark: form.value.remark,
      }),
    })
    modalOpen.value = false
    showMsg('网站配置已保存并生成 Nginx 站点文件')
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '保存网站失败'
  }
}

async function deleteSite(site: WebsiteWithRuntime) {
  if (!window.confirm(`确定删除网站「${site.name}」吗？（不会删除项目源码目录）`)) return
  try {
    await apiRequest(`/websites/${site.id}`, { method: 'DELETE' })
    showMsg(`已删除网站 ${site.name}`)
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '删除失败'
  }
}

async function runAction(site: WebsiteWithRuntime, action: 'start' | 'stop' | 'restart') {
  try {
    const msg = await apiRequest<string>(`/websites/${site.id}/action`, {
      method: 'POST',
      body: JSON.stringify({ action }),
    })
    showMsg(`${site.name}: ${msg}`)
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '进程操作失败'
  }
}

async function openNginxEditor(site: WebsiteWithRuntime) {
  try {
    nginxContent.value = await apiRequest<string>(`/websites/${site.id}/nginx`)
    nginxSiteId.value = site.id
    nginxSiteName.value = site.name
    nginxModalOpen.value = true
  } catch (err: any) {
    errorMsg.value = err?.message || '读取 Nginx 配置失败'
  }
}

async function saveNginxConfig() {
  try {
    await apiRequest(`/websites/${nginxSiteId.value}/nginx`, {
      method: 'PUT',
      body: JSON.stringify({ content: nginxContent.value }),
    })
    nginxModalOpen.value = false
    showMsg('已保存自定义 Nginx 站点配置')
  } catch (err: any) {
    errorMsg.value = err?.message || '保存 Nginx 配置失败'
  }
}

async function openLogs(site: WebsiteWithRuntime) {
  try {
    logsTitle.value = `${site.name} 运行日志`
    logsContent.value = await apiRequest<string>(`/websites/${site.id}/logs?lines=250`)
    logsModalOpen.value = true
  } catch (err: any) {
    errorMsg.value = err?.message || '读取日志失败'
  }
}

onMounted(loadAll)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <Globe class="h-5 w-5" style="color: var(--zm-accent)" />
          网站管理
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          精简聚焦：Rust / Go / Python（联动 uv）/ 反向代理 / HTML 纯静态 · 内置跨系统进程守护
        </p>
      </div>
      <div class="flex gap-2">
        <button class="zm-btn text-xs" @click="loadAll">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
        <button class="zm-btn zm-btn-primary text-xs" @click="openCreate()">
          <Plus class="h-3.5 w-3.5" /> 添加站点
        </button>
      </div>
    </div>

    <!-- Category Tabs -->
    <div class="flex flex-wrap gap-2">
      <button
        v-for="tab in KIND_TABS"
        :key="tab.id"
        class="zm-btn !py-1.5 text-xs"
        :class="{ 'zm-btn-primary': activeTab === tab.id }"
        @click="activeTab = tab.id"
      >
        {{ tab.label }}
      </button>
    </div>

    <div v-if="notice" class="zm-card px-3.5 py-2 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card px-3.5 py-2 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Sites Table -->
    <div class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">站点 / 域名</th>
              <th class="py-2.5 px-2">类型</th>
              <th class="py-2.5 px-2">运行状态</th>
              <th class="py-2.5 px-2">目录 / 目标</th>
              <th class="py-2.5 px-2">HTTPS / SSL</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="filteredSites.length === 0">
              <td colspan="6" class="py-12 text-center opacity-60">
                暂无网站项目，点击右上角「添加站点」快速创建 Rust / Go / Python / 反代 / 静态站点
              </td>
            </tr>
            <tr
              v-for="s in filteredSites"
              :key="s.id"
              class="border-b hover:bg-white/5"
              style="border-color: var(--zm-border)"
            >
              <td class="py-2.5 px-3">
                <div class="font-semibold">{{ s.name }}</div>
                <div class="font-mono text-[11px] opacity-75">{{ s.domains.join(', ') }}</div>
              </td>
              <td class="py-2.5 px-2">
                <span class="rounded px-2 py-0.5 text-[11px] font-medium" style="background: var(--zm-accent-soft); color: var(--zm-accent)">
                  {{ KIND_LABELS[s.kind] }}
                </span>
              </td>
              <td class="py-2.5 px-2">
                <span
                  v-if="s.runtime_status === 'running'"
                  class="inline-flex items-center gap-1 text-emerald-400 font-medium"
                >
                  ● 运行中 (PID {{ s.pid }})
                </span>
                <span
                  v-else-if="s.runtime_status === 'nginx'"
                  class="inline-flex items-center gap-1 opacity-80"
                >
                  ● Nginx 托管
                </span>
                <span v-else class="text-amber-400">○ 已停止</span>
              </td>
              <td class="py-2.5 px-2 max-w-xs truncate font-mono opacity-80" :title="s.root_dir">
                <div>{{ s.root_dir }}</div>
                <div v-if="s.kind === 'proxy'" class="text-[11px]" style="color: var(--zm-info)">
                  → {{ s.proxy_target }}
                </div>
                <div v-else-if="s.kind !== 'static'" class="text-[11px] opacity-70">
                  端口 :{{ s.listen_port }}
                </div>
              </td>
              <td class="py-2.5 px-2">
                <span v-if="s.ssl_cert_id" class="text-emerald-400">已绑定 SSL</span>
                <span v-else class="opacity-50">仅 HTTP</span>
              </td>
              <td class="py-2.5 px-3 text-right space-x-1 whitespace-nowrap">
                <template v-if="s.kind === 'rust' || s.kind === 'go' || s.kind === 'python'">
                  <button
                    v-if="s.runtime_status !== 'running'"
                    class="zm-btn !py-1 !px-2 text-[11px]"
                    title="启动进程"
                    @click="runAction(s, 'start')"
                  >
                    <Play class="h-3 w-3 text-emerald-400" />
                  </button>
                  <button
                    v-else
                    class="zm-btn !py-1 !px-2 text-[11px]"
                    title="停止进程"
                    @click="runAction(s, 'stop')"
                  >
                    <Square class="h-3 w-3 text-rose-400" />
                  </button>
                  <button class="zm-btn !py-1 !px-2 text-[11px]" title="重启" @click="runAction(s, 'restart')">
                    <RotateCw class="h-3 w-3" />
                  </button>
                  <button class="zm-btn !py-1 !px-2 text-[11px]" title="运行日志" @click="openLogs(s)">
                    <ScrollText class="h-3 w-3" />
                  </button>
                </template>
                <button class="zm-btn !py-1 !px-2 text-[11px]" title="Nginx 配置" @click="openNginxEditor(s)">
                  <FileCode2 class="h-3 w-3" /> 配置
                </button>
                <button
                  class="zm-btn !py-1 !px-2 text-[11px]"
                  title="打开目录终端"
                  @click="router.push({ path: '/terminal', query: { cwd: s.root_dir } })"
                >
                  <Terminal class="h-3 w-3" />
                </button>
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="openEdit(s)">设置</button>
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="deleteSite(s)">
                  <Trash2 class="h-3 w-3" />
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Create / Edit Modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4 overflow-y-auto">
      <div class="zm-card w-full max-w-2xl p-5 space-y-4 my-auto">
        <div class="flex items-center justify-between">
          <div class="font-semibold text-sm">{{ form.id ? '编辑网站' : '添加网站项目' }}</div>
          <button class="opacity-60 hover:opacity-100 cursor-pointer" @click="modalOpen = false">
            <X class="h-4 w-4" />
          </button>
        </div>

        <div class="grid grid-cols-1 sm:grid-cols-2 gap-3 text-xs">
          <div>
            <label class="block mb-1 opacity-75">项目名称</label>
            <input v-model="form.name" class="zm-input" placeholder="例如：my-rust-api" />
          </div>
          <div>
            <label class="block mb-1 opacity-75">项目类型</label>
            <select v-model="form.kind" class="zm-input">
              <option value="rust">Rust 项目（单二进制 + 进程守护）</option>
              <option value="go">Go 项目（单二进制 + 进程守护）</option>
              <option value="python">Python 项目（uv 虚拟环境 + 进程守护）</option>
              <option value="proxy">反向代理</option>
              <option value="static">HTML 纯静态</option>
            </select>
          </div>
          <div class="sm:col-span-2">
            <label class="block mb-1 opacity-75">绑定域名（每行或逗号分隔）</label>
            <input v-model="form.domainsText" class="zm-input font-mono" placeholder="api.example.com" />
          </div>
          <div class="sm:col-span-2">
            <label class="block mb-1 opacity-75">项目根目录（不存在时自动创建）</label>
            <input v-model="form.root_dir" class="zm-input font-mono" placeholder="/opt/apps/demo" />
          </div>

          <template v-if="form.kind === 'rust' || form.kind === 'go' || form.kind === 'python'">
            <div>
              <label class="block mb-1 opacity-75">后端监听端口（供 Nginx 反代）</label>
              <input v-model.number="form.listen_port" type="number" class="zm-input font-mono" />
            </div>
            <div v-if="form.kind === 'python'">
              <label class="block mb-1 opacity-75">Python 虚拟环境（联动 uv）</label>
              <select v-model="form.python_venv" class="zm-input">
                <option value="">使用系统默认 Python</option>
                <option v-for="v in pythonVenvs" :key="v.id" :value="v.path">
                  {{ v.name }} ({{ v.python_version }}) - {{ v.path }}
                </option>
              </select>
            </div>
            <div class="sm:col-span-2">
              <label class="block mb-1 opacity-75">启动命令（直接参数执行，无需 bash 包装）</label>
              <input v-model="form.start_cmd" class="zm-input font-mono" placeholder="./server --port 8080" />
            </div>
            <div class="sm:col-span-2">
              <label class="block mb-1 opacity-75">环境变量（每行 KEY=VALUE）</label>
              <textarea v-model="form.env_vars" rows="2" class="zm-input font-mono" placeholder="RUST_LOG=info" />
            </div>
          </template>

          <div v-if="form.kind === 'proxy'" class="sm:col-span-2">
            <label class="block mb-1 opacity-75">上游目标地址 (proxy_pass)</label>
            <input v-model="form.proxy_target" class="zm-input font-mono" placeholder="http://127.0.0.1:8080" />
          </div>

          <div>
            <label class="block mb-1 opacity-75">绑定 SSL 证书（独立证书中心）</label>
            <select v-model="form.ssl_cert_id" class="zm-input">
              <option value="">暂不绑定（仅 HTTP）</option>
              <option v-for="c in sslCerts" :key="c.id" :value="c.id">
                {{ c.name }} ({{ c.domains.join(', ') }})
              </option>
            </select>
          </div>
          <div class="flex flex-wrap items-center gap-4 pt-5">
            <label class="flex items-center gap-1.5 cursor-pointer">
              <input v-model="form.force_https" type="checkbox" /> 强制 HTTPS
            </label>
            <label v-if="form.kind !== 'static'" class="flex items-center gap-1.5 cursor-pointer">
              <input v-model="form.websocket_enabled" type="checkbox" /> WebSocket
            </label>
            <label v-if="form.kind === 'static'" class="flex items-center gap-1.5 cursor-pointer">
              <input v-model="form.spa_fallback" type="checkbox" /> SPA 路由回退
            </label>
          </div>
        </div>

        <div class="flex justify-end gap-2 pt-2">
          <button class="zm-btn" @click="modalOpen = false">取消</button>
          <button class="zm-btn zm-btn-primary" @click="saveSite">保存站点</button>
        </div>
      </div>
    </div>

    <!-- Nginx Config Editor Modal -->
    <div v-if="nginxModalOpen" class="fixed inset-0 z-50 flex flex-col p-4 bg-black/70">
      <div class="zm-card flex-1 flex flex-col overflow-hidden">
        <div class="flex items-center justify-between px-4 py-2.5 border-b" style="border-color: var(--zm-border)">
          <span class="text-xs font-semibold">Nginx 站点配置 · {{ nginxSiteName }}</span>
          <div class="flex gap-2">
            <button class="zm-btn zm-btn-primary !py-1 text-xs" @click="saveNginxConfig">保存配置</button>
            <button class="zm-btn !py-1 text-xs" @click="nginxModalOpen = false">关闭</button>
          </div>
        </div>
        <div class="flex-1 min-h-0 p-2">
          <CodeEditor v-model="nginxContent" filename="site.conf" :dark="theme.isDark()" @save="saveNginxConfig" />
        </div>
      </div>
    </div>

    <!-- Site Logs Modal -->
    <div v-if="logsModalOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div class="zm-card w-full max-w-4xl h-[75vh] flex flex-col overflow-hidden">
        <div class="flex items-center justify-between px-4 py-2.5 border-b" style="border-color: var(--zm-border)">
          <span class="text-xs font-semibold">{{ logsTitle }}</span>
          <button class="zm-btn !py-1 text-xs" @click="logsModalOpen = false">关闭</button>
        </div>
        <pre
          class="flex-1 overflow-auto p-4 text-xs font-mono m-0"
          style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
        >{{ logsContent }}</pre>
      </div>
    </div>
  </div>
</template>
