<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AlertTriangle, Box, CheckCircle2, Database, Download, Plus, RefreshCw, Shield, Store, Terminal, Trash2 } from 'lucide-vue-next'
import { apiRequest } from '@/api/client'

interface StoreAppItem {
  id: string
  name: string
  category: string
  description: string
  min_mem_mb: number
  supported_os: string[]
  install_modes: string[]
  installed: boolean
  running: boolean
  version: string
  has_manager: boolean
  compat_warning?: string | null
}

interface NginxSecurityConfig {
  hide_version: boolean
  gzip_enabled: boolean
  client_max_body_mb: number
  rate_limit_enabled: boolean
  rate_limit_rps: number
  rate_limit_burst: number
  block_sensitive_paths: boolean
  block_bad_ua: boolean
  blocked_ips: string[]
  custom_rules: string
}

interface PythonVenvItem {
  id: string
  name: string
  path: string
  python_version: string
  index_url: string
}

const router = useRouter()
const activeTab = ref<'catalog' | 'nginx' | 'uv' | 'db'>('catalog')
const apps = ref<StoreAppItem[]>([])
const dockerNestingSupported = ref(true)
const dockerEnvNotice = ref('')
const notice = ref('')
const errorMsg = ref('')
const busy = ref(false)

// Redis / PostgreSQL / MongoDB service configs
const activeDbSub = ref<'redis' | 'postgresql' | 'mongodb'>('redis')
const redisCfg = ref({
  bind: '127.0.0.1',
  port: 6379,
  password: '',
  maxmemory_mb: 64,
  maxmemory_policy: 'allkeys-lru',
  appendonly: true,
})
const redisCompose = ref('')
const pgCfg = ref({
  bind: '127.0.0.1',
  port: 5432,
  superuser: 'postgres',
  password: '',
  default_db: 'appdb',
  shared_buffers_mb: 32,
  max_connections: 40,
})
const pgCompose = ref('')
const mongoCfg = ref({
  bind: '127.0.0.1',
  port: 27017,
  root_user: 'admin',
  root_password: '',
  cache_size_gb: 0.25,
  image_tag: '4.4',
})
const mongoCompose = ref('')

// Nginx state
const nginxCfg = ref<NginxSecurityConfig>({
  hide_version: true,
  gzip_enabled: true,
  client_max_body_mb: 50,
  rate_limit_enabled: true,
  rate_limit_rps: 20,
  rate_limit_burst: 40,
  block_sensitive_paths: true,
  block_bad_ua: true,
  blocked_ips: [],
  custom_rules: '',
})
const blockedIpsText = ref('')
const nginxRendered = ref('')
const nginxConfPath = ref('')

// uv state
const uvInstalled = ref(false)
const uvVersion = ref('')
const uvPythons = ref('')
const venvs = ref<PythonVenvItem[]>([])
const installPyVer = ref('3.12')
const newVenv = ref({
  name: '',
  path: '/opt/venvs/default',
  python_version: '3.12',
  index_url: 'https://pypi.tuna.tsinghua.edu.cn/simple',
})
const pipForm = ref({
  venv_path: '',
  packagesText: '',
  index_url: 'https://pypi.tuna.tsinghua.edu.cn/simple',
})

function showMsg(msg: string) {
  notice.value = msg
  errorMsg.value = ''
}

async function loadAll() {
  try {
    const [a, n, u, r, p, m, dk] = await Promise.all([
      apiRequest<StoreAppItem[]>('/store/apps'),
      apiRequest<{ config: NginxSecurityConfig; rendered_conf: string; conf_path: string }>('/store/nginx/security'),
      apiRequest<{ installed: boolean; version: string; installed_pythons: string; venvs: PythonVenvItem[] }>('/store/uv/overview'),
      apiRequest<{ config: typeof redisCfg.value; rendered_compose: string }>('/store/services/redis'),
      apiRequest<{ config: typeof pgCfg.value; rendered_compose: string }>('/store/services/postgresql'),
      apiRequest<{ config: typeof mongoCfg.value; rendered_compose: string }>('/store/services/mongodb'),
      apiRequest<{ env?: { docker_nesting_supported: boolean; virt_label: string; docker_nesting_reason: string } }>('/docker/status').catch(() => null),
    ])
    apps.value = a
    if (dk?.env) {
      dockerNestingSupported.value = dk.env.docker_nesting_supported
      dockerEnvNotice.value = dk.env.docker_nesting_supported
        ? ''
        : `当前为「${dk.env.virt_label}」，不支持嵌套运行 Docker。请优先在「应用列表」中使用原生包安装，或仅将生成的配置保存用于其他宿主机。`
    }
    nginxCfg.value = n.config
    blockedIpsText.value = (n.config.blocked_ips || []).join('\n')
    nginxRendered.value = n.rendered_conf
    nginxConfPath.value = n.conf_path
    uvInstalled.value = u.installed
    uvVersion.value = u.version
    uvPythons.value = u.installed_pythons
    venvs.value = u.venvs
    redisCfg.value = r.config
    redisCompose.value = r.rendered_compose
    pgCfg.value = p.config
    pgCompose.value = p.rendered_compose
    mongoCfg.value = m.config
    mongoCompose.value = m.rendered_compose
  } catch (err: any) {
    errorMsg.value = err?.message || '加载软件商店失败'
  }
}

async function saveDbService(svc: 'redis' | 'postgresql' | 'mongodb') {
  errorMsg.value = ''
  try {
    const body = svc === 'redis' ? redisCfg.value : svc === 'postgresql' ? pgCfg.value : mongoCfg.value
    const res = await apiRequest<{ rendered_compose: string; conf_path: string }>(`/store/services/${svc}`, {
      method: 'PUT',
      body: JSON.stringify(body),
    })
    if (svc === 'redis') redisCompose.value = res.rendered_compose
    if (svc === 'postgresql') pgCompose.value = res.rendered_compose
    if (svc === 'mongodb') mongoCompose.value = res.rendered_compose
    showMsg(`已保存 ${svc} 配置至 ${res.conf_path}`)
  } catch (err: any) {
    errorMsg.value = err?.message || '保存配置失败'
  }
}

async function sendToDockerWorkspace(svc: 'redis' | 'postgresql' | 'mongodb') {
  await saveDbService(svc)
  try {
    const proj = await apiRequest<{ name: string; work_dir: string }>('/store/services/to-docker', {
      method: 'POST',
      body: JSON.stringify({ service: svc }),
    })
    showMsg(`已一键生成 Docker 工作区「${proj.name}」(${proj.work_dir})，即将前往 Docker 工作区部署...`)
    setTimeout(() => router.push('/docker'), 1100)
  } catch (err: any) {
    errorMsg.value = err?.message || '生成 Docker 工作区失败'
  }
}

async function installApp(app: StoreAppItem) {
  busy.value = true
  errorMsg.value = ''
  try {
    const out = await apiRequest<string>(`/store/apps/${app.id}/install`, { method: 'POST' })
    showMsg(`${app.name} 安装完成：\n${out}`)
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '安装失败'
  } finally {
    busy.value = false
  }
}

async function saveNginxSecurity() {
  errorMsg.value = ''
  const payload: NginxSecurityConfig = {
    ...nginxCfg.value,
    blocked_ips: blockedIpsText.value
      .split(/[\n,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean),
  }
  try {
    const res = await apiRequest<{ config: NginxSecurityConfig; rendered_conf: string; conf_path: string }>(
      '/store/nginx/security',
      { method: 'PUT', body: JSON.stringify(payload) },
    )
    nginxRendered.value = res.rendered_conf
    nginxConfPath.value = res.conf_path
    showMsg(`已保存 Nginx 性能与轻量安全规则至 ${res.conf_path}`)
  } catch (err: any) {
    errorMsg.value = err?.message || '保存 Nginx 设置失败'
  }
}

async function nginxCtrl(action: string) {
  try {
    const out = await apiRequest<string>('/store/nginx/control', {
      method: 'POST',
      body: JSON.stringify({ action }),
    })
    showMsg(out)
  } catch (err: any) {
    errorMsg.value = err?.message || '执行 Nginx 命令失败'
  }
}

async function installPython() {
  busy.value = true
  errorMsg.value = ''
  try {
    const out = await apiRequest<string>('/store/uv/python/install', {
      method: 'POST',
      body: JSON.stringify({ version: installPyVer.value }),
    })
    showMsg(out || `已安装 Python ${installPyVer.value}`)
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '安装 Python 失败'
  } finally {
    busy.value = false
  }
}

async function createVenv() {
  busy.value = true
  errorMsg.value = ''
  try {
    await apiRequest('/store/uv/venvs', {
      method: 'POST',
      body: JSON.stringify(newVenv.value),
    })
    showMsg(`已创建虚拟环境 ${newVenv.value.name}`)
    newVenv.value.name = ''
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '创建虚拟环境失败'
  } finally {
    busy.value = false
  }
}

async function deleteVenv(id: string) {
  await apiRequest(`/store/uv/venvs/${id}`, { method: 'DELETE' })
  await loadAll()
}

async function pipInstall() {
  const packages = pipForm.value.packagesText
    .split(/[\n,\s]+/)
    .map((s) => s.trim())
    .filter(Boolean)
  busy.value = true
  errorMsg.value = ''
  try {
    const out = await apiRequest<string>('/store/uv/pip/install', {
      method: 'POST',
      body: JSON.stringify({
        venv_path: pipForm.value.venv_path,
        packages,
        index_url: pipForm.value.index_url,
      }),
    })
    showMsg(out || '依赖包安装成功')
  } catch (err: any) {
    errorMsg.value = err?.message || '依赖安装失败'
  } finally {
    busy.value = false
  }
}

onMounted(loadAll)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <Store class="h-5 w-5" style="color: var(--zm-accent)" />
          软件商店与运行环境
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          声明式 TOML 软件清单（支持自定义扩展）· Nginx 轻量安全设置 · Python uv 极速管理器
        </p>
      </div>
      <button class="zm-btn text-xs" @click="loadAll">
        <RefreshCw class="h-3.5 w-3.5" /> 刷新探测
      </button>
    </div>

    <div class="flex flex-wrap gap-2">
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'catalog' }" @click="activeTab = 'catalog'">
        全部软件 ({{ apps.length }})
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'nginx' }" @click="activeTab = 'nginx'">
        <Shield class="h-3.5 w-3.5" /> Nginx 安全与性能设置
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'uv' }" @click="activeTab = 'uv'">
        <Terminal class="h-3.5 w-3.5" /> Python 管理器 (uv)
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeTab === 'db' }" @click="activeTab = 'db'">
        <Database class="h-3.5 w-3.5" /> Redis / PostgreSQL / MongoDB
      </button>
    </div>

    <div v-if="notice" class="zm-card p-3 text-xs whitespace-pre-wrap" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card p-3 text-xs whitespace-pre-wrap" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Tab 1: Catalog -->
    <div v-if="activeTab === 'catalog'" class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
      <div v-for="app in apps" :key="app.id" class="zm-card p-4 flex flex-col justify-between gap-3">
        <div class="space-y-2">
          <div class="flex items-start justify-between gap-2">
            <div>
              <div class="font-semibold text-sm">{{ app.name }}</div>
              <div class="text-[11px] opacity-60">{{ app.category }} · 最低建议 {{ app.min_mem_mb }}MB 内存</div>
            </div>
            <span
              class="rounded-full px-2 py-0.5 text-[10px] font-semibold"
              :style="
                app.installed
                  ? { background: 'var(--zm-accent-soft)', color: 'var(--zm-accent)' }
                  : { background: 'var(--zm-surface-elevated)', color: 'var(--zm-text-muted)' }
              "
            >
              {{ app.installed ? '已安装' : '未安装' }}
            </span>
          </div>
          <p class="text-xs opacity-80 leading-relaxed">{{ app.description }}</p>
          <div v-if="app.version" class="text-[11px] font-mono opacity-75 truncate">{{ app.version }}</div>
          <div
            v-if="app.compat_warning"
            class="flex items-start gap-1.5 rounded-lg p-2 text-[11px]"
            style="background: rgba(245, 158, 11, 0.12); color: var(--zm-warning)"
          >
            <AlertTriangle class="h-3.5 w-3.5 shrink-0 mt-0.5" />
            <span>{{ app.compat_warning }}</span>
          </div>
        </div>
        <div class="flex items-center justify-between pt-2 border-t" style="border-color: var(--zm-border)">
          <span class="text-[11px] opacity-60">模式: {{ app.install_modes.join(' / ') }}</span>
          <div class="flex gap-1.5">
            <button
              v-if="app.id === 'nginx'"
              class="zm-btn zm-btn-primary !py-1 !px-2.5 text-xs"
              @click="activeTab = 'nginx'"
            >
              打开设置
            </button>
            <button
              v-else-if="app.id === 'python-uv'"
              class="zm-btn zm-btn-primary !py-1 !px-2.5 text-xs"
              @click="activeTab = 'uv'"
            >
              打开管理
            </button>
            <button
              v-else-if="app.id === 'redis' || app.id === 'postgresql' || app.id === 'mongodb'"
              class="zm-btn zm-btn-primary !py-1 !px-2.5 text-xs"
              @click="activeDbSub = app.id; activeTab = 'db'"
            >
              配置与部署
            </button>
            <button
              v-if="!app.installed && app.id !== 'python-uv'"
              class="zm-btn !py-1 !px-2.5 text-xs"
              :disabled="busy"
              @click="installApp(app)"
            >
              <Download class="h-3 w-3" /> 安装
            </button>
          </div>
        </div>
      </div>
    </div>

    <!-- Tab 2: Nginx Security & Perf Manager -->
    <div v-if="activeTab === 'nginx'" class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <div class="zm-card p-5 space-y-4 text-xs">
        <div class="flex items-center justify-between">
          <div class="font-semibold text-sm">Nginx 轻量安全防护（替代复杂 WAF）</div>
          <div class="flex gap-1.5">
            <button class="zm-btn !py-1 !px-2 text-[11px]" @click="nginxCtrl('test')">测试配置 (-t)</button>
            <button class="zm-btn !py-1 !px-2 text-[11px]" @click="nginxCtrl('reload')">热重载</button>
          </div>
        </div>

        <div class="space-y-2.5">
          <label class="flex items-center gap-2 cursor-pointer">
            <input v-model="nginxCfg.hide_version" type="checkbox" /> 隐藏 Nginx 版本号 (server_tokens off)
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input v-model="nginxCfg.gzip_enabled" type="checkbox" /> 开启 Gzip 压缩加速
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input v-model="nginxCfg.block_sensitive_paths" type="checkbox" /> 拦截恶意探测路径 (.env / .git / wp-login / phpmyadmin / .sql)
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input v-model="nginxCfg.block_bad_ua" type="checkbox" /> 拦截常见漏洞扫描器 User-Agent (sqlmap / nikto / nmap 等)
          </label>
          <label class="flex items-center gap-2 cursor-pointer">
            <input v-model="nginxCfg.rate_limit_enabled" type="checkbox" /> 开启高频 CC 请求限流 (limit_req)
          </label>
        </div>

        <div class="grid grid-cols-3 gap-2">
          <div>
            <label class="block mb-1 opacity-75">单请求体上限 (MB)</label>
            <input v-model.number="nginxCfg.client_max_body_mb" type="number" class="zm-input" />
          </div>
          <div>
            <label class="block mb-1 opacity-75">限流速率 (req/s)</label>
            <input v-model.number="nginxCfg.rate_limit_rps" type="number" class="zm-input" />
          </div>
          <div>
            <label class="block mb-1 opacity-75">突发队列 (burst)</label>
            <input v-model.number="nginxCfg.rate_limit_burst" type="number" class="zm-input" />
          </div>
        </div>

        <div>
          <label class="block mb-1 opacity-75">Nginx IP 黑名单（每行一个 IP 或 CIDR）</label>
          <textarea v-model="blockedIpsText" rows="3" class="zm-input font-mono" placeholder="192.0.2.10&#10;198.51.100.0/24" />
        </div>

        <div>
          <label class="block mb-1 opacity-75">追加自定义规则</label>
          <textarea v-model="nginxCfg.custom_rules" rows="2" class="zm-input font-mono" placeholder="# 自定义 location / add_header 等" />
        </div>

        <button class="zm-btn zm-btn-primary text-xs" @click="saveNginxSecurity">
          <CheckCircle2 class="h-3.5 w-3.5" /> 保存并生成 security.conf
        </button>
      </div>

      <div class="zm-card p-5 flex flex-col gap-2">
        <div class="text-xs font-semibold">生成的安全配置预览</div>
        <div class="text-[11px] font-mono opacity-70 truncate">{{ nginxConfPath }}</div>
        <pre
          class="flex-1 overflow-auto rounded-lg p-3 text-xs font-mono m-0"
          style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
        >{{ nginxRendered }}</pre>
      </div>
    </div>

    <!-- Tab 3: Python uv Manager -->
    <div v-if="activeTab === 'uv'" class="space-y-4 text-xs">
      <div class="zm-card p-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <div class="font-semibold text-sm">
            uv 状态：
            <span :style="{ color: uvInstalled ? 'var(--zm-accent)' : 'var(--zm-warning)' }">
              {{ uvInstalled ? uvVersion : '未检测到 uv' }}
            </span>
          </div>
          <div class="opacity-70 mt-0.5">
            Linux 服务器一行安装：<code>curl -LsSf https://astral.sh/uv/install.sh | sh</code>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <input v-model="installPyVer" class="zm-input !w-28 font-mono" placeholder="3.12" />
          <button class="zm-btn zm-btn-primary text-xs" :disabled="busy || !uvInstalled" @click="installPython">
            <Download class="h-3.5 w-3.5" /> 安装 Python 版本
          </button>
        </div>
      </div>

      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <!-- Create Venv -->
        <div class="zm-card p-4 space-y-3">
          <div class="font-semibold text-sm">创建 Python 虚拟环境 (uv venv)</div>
          <div class="grid grid-cols-2 gap-2">
            <input v-model="newVenv.name" class="zm-input" placeholder="环境名称（如 fastapi-env）" />
            <input v-model="newVenv.python_version" class="zm-input font-mono" placeholder="Python 版本（如 3.12）" />
          </div>
          <input v-model="newVenv.path" class="zm-input font-mono" placeholder="虚拟环境目录路径" />
          <input v-model="newVenv.index_url" class="zm-input font-mono" placeholder="PyPI 镜像源（可选）" />
          <button class="zm-btn zm-btn-primary text-xs" :disabled="busy || !uvInstalled" @click="createVenv">
            <Plus class="h-3.5 w-3.5" /> 创建虚拟环境
          </button>

          <div class="pt-2 space-y-2">
            <div class="font-semibold">已纳管虚拟环境 ({{ venvs.length }})</div>
            <div v-if="venvs.length === 0" class="opacity-60">暂无记录</div>
            <div
              v-for="v in venvs"
              :key="v.id"
              class="rounded-lg p-2.5 flex items-center justify-between gap-2"
              style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
            >
              <div class="min-w-0">
                <div class="font-semibold">{{ v.name }} (Python {{ v.python_version }})</div>
                <div class="font-mono text-[11px] opacity-70 truncate">{{ v.path }}</div>
              </div>
              <div class="flex gap-1">
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="pipForm.venv_path = v.path">装包</button>
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="deleteVenv(v.id)">
                  <Trash2 class="h-3 w-3" />
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- Pip Install & Installed Pythons -->
        <div class="zm-card p-4 space-y-3">
          <div class="font-semibold text-sm">虚拟环境极速装包 (uv pip install)</div>
          <input v-model="pipForm.venv_path" class="zm-input font-mono" placeholder="目标虚拟环境路径（可点击左侧「装包」自动填入）" />
          <input v-model="pipForm.packagesText" class="zm-input font-mono" placeholder="包名（空格分隔，如 fastapi uvicorn httpx）" />
          <input v-model="pipForm.index_url" class="zm-input font-mono" placeholder="镜像源 URL" />
          <button class="zm-btn zm-btn-primary text-xs" :disabled="busy || !uvInstalled" @click="pipInstall">
            安装依赖包
          </button>

          <div v-if="uvPythons" class="pt-2 space-y-1.5">
            <div class="font-semibold">本机已安装 Python 列表</div>
            <pre
              class="p-2.5 rounded-lg font-mono text-[11px] overflow-auto max-h-44 m-0"
              style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
            >{{ uvPythons }}</pre>
          </div>
        </div>
      </div>
    </div>

    <!-- Tab 4: Redis / PostgreSQL / MongoDB Config & Docker Workspace Generator -->
    <div v-if="activeTab === 'db'" class="space-y-4 text-xs">
      <div
        v-if="!dockerNestingSupported && dockerEnvNotice"
        class="zm-card p-3.5 text-xs"
        style="border-color: var(--zm-danger); color: var(--zm-danger)"
      >
        {{ dockerEnvNotice }}
      </div>
      <div class="flex gap-2">
        <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeDbSub === 'redis' }" @click="activeDbSub = 'redis'">
          Redis
        </button>
        <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeDbSub === 'postgresql' }" @click="activeDbSub = 'postgresql'">
          PostgreSQL
        </button>
        <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': activeDbSub === 'mongodb' }" @click="activeDbSub = 'mongodb'">
          MongoDB
        </button>
      </div>

      <!-- Redis -->
      <div v-if="activeDbSub === 'redis'" class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <div class="zm-card p-5 space-y-3">
          <div class="font-semibold text-sm">Redis 服务参数配置（支持小内存防 OOM 上限）</div>
          <div class="grid grid-cols-2 gap-2.5">
            <div><label class="block mb-1 opacity-75">监听地址</label><input v-model="redisCfg.bind" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">端口</label><input v-model.number="redisCfg.port" type="number" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">最大内存上限 (MB)</label><input v-model.number="redisCfg.maxmemory_mb" type="number" class="zm-input font-mono" /></div>
            <div>
              <label class="block mb-1 opacity-75">内存淘汰策略</label>
              <select v-model="redisCfg.maxmemory_policy" class="zm-input">
                <option value="allkeys-lru">allkeys-lru (推荐)</option>
                <option value="volatile-lru">volatile-lru</option>
                <option value="noeviction">noeviction</option>
              </select>
            </div>
          </div>
          <div><label class="block mb-1 opacity-75">访问密码 (requirepass，留空无密码)</label><input v-model="redisCfg.password" type="password" class="zm-input font-mono" /></div>
          <label class="flex items-center gap-2 cursor-pointer"><input v-model="redisCfg.appendonly" type="checkbox" /> 开启 AOF 持久化</label>
          <div class="flex gap-2 pt-1">
            <button class="zm-btn" @click="saveDbService('redis')">保存配置</button>
            <button
              class="zm-btn zm-btn-primary"
              :disabled="!dockerNestingSupported"
              :title="!dockerNestingSupported ? '当前容器环境不支持嵌套 Docker' : ''"
              @click="sendToDockerWorkspace('redis')"
            >
              <Box class="h-3.5 w-3.5" /> 一键生成 Docker 工作区
            </button>
          </div>
        </div>
        <div class="zm-card p-5 flex flex-col gap-2">
          <div class="font-semibold">生成的 Redis Compose 模板</div>
          <pre class="flex-1 overflow-auto rounded-lg p-3 font-mono text-[11px] m-0" style="background: var(--zm-term-bg); color: var(--zm-term-fg)">{{ redisCompose }}</pre>
        </div>
      </div>

      <!-- PostgreSQL -->
      <div v-if="activeDbSub === 'postgresql'" class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <div class="zm-card p-5 space-y-3">
          <div class="font-semibold text-sm">PostgreSQL 参数调优（默认适配低配 VPS）</div>
          <div class="grid grid-cols-2 gap-2.5">
            <div><label class="block mb-1 opacity-75">监听地址</label><input v-model="pgCfg.bind" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">端口</label><input v-model.number="pgCfg.port" type="number" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">管理员用户名</label><input v-model="pgCfg.superuser" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">默认数据库名</label><input v-model="pgCfg.default_db" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">shared_buffers (MB)</label><input v-model.number="pgCfg.shared_buffers_mb" type="number" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">max_connections</label><input v-model.number="pgCfg.max_connections" type="number" class="zm-input font-mono" /></div>
          </div>
          <div><label class="block mb-1 opacity-75">管理员密码</label><input v-model="pgCfg.password" type="password" class="zm-input font-mono" placeholder="强烈建议设置强密码" /></div>
          <div class="flex gap-2 pt-1">
            <button class="zm-btn" @click="saveDbService('postgresql')">保存配置</button>
            <button
              class="zm-btn zm-btn-primary"
              :disabled="!dockerNestingSupported"
              :title="!dockerNestingSupported ? '当前容器环境不支持嵌套 Docker' : ''"
              @click="sendToDockerWorkspace('postgresql')"
            >
              <Box class="h-3.5 w-3.5" /> 一键生成 Docker 工作区
            </button>
          </div>
        </div>
        <div class="zm-card p-5 flex flex-col gap-2">
          <div class="font-semibold">生成的 PostgreSQL Compose 模板</div>
          <pre class="flex-1 overflow-auto rounded-lg p-3 font-mono text-[11px] m-0" style="background: var(--zm-term-bg); color: var(--zm-term-fg)">{{ pgCompose }}</pre>
        </div>
      </div>

      <!-- MongoDB -->
      <div v-if="activeDbSub === 'mongodb'" class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <div class="zm-card p-5 space-y-3">
          <div class="font-semibold text-sm">MongoDB 配置（兼容无 AVX NAT 小鸡）</div>
          <div class="grid grid-cols-2 gap-2.5">
            <div><label class="block mb-1 opacity-75">监听地址</label><input v-model="mongoCfg.bind" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">端口</label><input v-model.number="mongoCfg.port" type="number" class="zm-input font-mono" /></div>
            <div><label class="block mb-1 opacity-75">Root 用户名</label><input v-model="mongoCfg.root_user" class="zm-input font-mono" /></div>
            <div>
              <label class="block mb-1 opacity-75">镜像版本标签</label>
              <select v-model="mongoCfg.image_tag" class="zm-input">
                <option value="4.4">mongo:4.4（无需 CPU AVX，适配 NAT 小鸡）</option>
                <option value="7">mongo:7（现代版本，需 CPU 支持 AVX）</option>
              </select>
            </div>
            <div><label class="block mb-1 opacity-75">WiredTiger 缓存上限 (GB)</label><input v-model.number="mongoCfg.cache_size_gb" type="number" step="0.05" min="0.1" class="zm-input font-mono" /></div>
          </div>
          <div><label class="block mb-1 opacity-75">Root 密码</label><input v-model="mongoCfg.root_password" type="password" class="zm-input font-mono" /></div>
          <div class="flex gap-2 pt-1">
            <button class="zm-btn" @click="saveDbService('mongodb')">保存配置</button>
            <button
              class="zm-btn zm-btn-primary"
              :disabled="!dockerNestingSupported"
              :title="!dockerNestingSupported ? '当前容器环境不支持嵌套 Docker' : ''"
              @click="sendToDockerWorkspace('mongodb')"
            >
              <Box class="h-3.5 w-3.5" /> 一键生成 Docker 工作区
            </button>
          </div>
        </div>
        <div class="zm-card p-5 flex flex-col gap-2">
          <div class="font-semibold">生成的 MongoDB Compose 模板</div>
          <pre class="flex-1 overflow-auto rounded-lg p-3 font-mono text-[11px] m-0" style="background: var(--zm-term-bg); color: var(--zm-term-fg)">{{ mongoCompose }}</pre>
        </div>
      </div>
    </div>
  </div>
</template>
