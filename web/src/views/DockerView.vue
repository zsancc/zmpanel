<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  Box,
  Download,
  Edit3,
  FolderOpen,
  Layers,
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

interface RuntimeEnvInfo {
  virt_type: string
  virt_label: string
  inside_container: boolean
  docker_nesting_supported: boolean
  docker_mode: string
  docker_nesting_reason: string
}

interface DockerStatus {
  available: boolean
  version: string
  compose_available: boolean
  message: string
  env?: RuntimeEnvInfo
}

interface DockerProject {
  id: string
  name: string
  work_dir: string
  compose_file: string
  deploy_cmd: string
  description: string
}

interface DockerContainer {
  id: string
  name: string
  image: string
  state: string
  status: string
  ports: string
  cpu_pct: string
  mem_usage: string
}

interface DockerImage {
  id: string
  repository: string
  tag: string
  size: string
  created: string
}

const router = useRouter()
const theme = useThemeStore()

const tab = ref<'projects' | 'containers' | 'images'>('projects')
const status = ref<DockerStatus | null>(null)
const projects = ref<DockerProject[]>([])
const containers = ref<DockerContainer[]>([])
const images = ref<DockerImage[]>([])
const busy = ref(false)
const installingDocker = ref(false)
const outputLog = ref('')
const errorMsg = ref('')

// Create / Edit Project Modal
const modalOpen = ref(false)
const form = ref({
  id: '' as string | undefined,
  name: '',
  work_dir: '',
  compose_file: 'docker-compose.yml',
  deploy_cmd: 'docker compose up -d',
  description: '',
  compose_content: '',
  env_content: '',
})

// Inline Compose Editor Modal
const editComposeOpen = ref(false)
const editingComposePath = ref('')
const editingComposeText = ref('')

// Container logs modal
const logsOpen = ref(false)
const logsTitle = ref('')
const logsText = ref('')

async function loadAll() {
  errorMsg.value = ''
  try {
    const [st, projs] = await Promise.all([
      apiRequest<DockerStatus>('/docker/status'),
      apiRequest<DockerProject[]>('/docker/projects'),
    ])
    status.value = st
    projects.value = projs
    if (st.available) {
      const [cs, imgs] = await Promise.all([
        apiRequest<DockerContainer[]>('/docker/containers?all=true').catch(() => []),
        apiRequest<DockerImage[]>('/docker/images').catch(() => []),
      ])
      containers.value = cs
      images.value = imgs
    }
  } catch (err: any) {
    errorMsg.value = err?.message || '加载 Docker 状态失败'
  }
}

function openCreateProject() {
  form.value = {
    id: undefined,
    name: '',
    work_dir: '',
    compose_file: 'docker-compose.yml',
    deploy_cmd: 'docker compose up -d',
    description: '',
    compose_content: `services:\n  app:\n    image: nginx:alpine\n    restart: unless-stopped\n    ports:\n      - "8080:80"\n`,
    env_content: '',
  }
  modalOpen.value = true
}

async function saveProject() {
  errorMsg.value = ''
  try {
    await apiRequest('/docker/projects', {
      method: 'POST',
      body: JSON.stringify({
        id: form.value.id,
        name: form.value.name,
        work_dir: form.value.work_dir || undefined,
        compose_file: form.value.compose_file,
        deploy_cmd: form.value.deploy_cmd,
        description: form.value.description,
        compose_content: form.value.compose_content || undefined,
        env_content: form.value.env_content || undefined,
      }),
    })
    modalOpen.value = false
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '保存 Docker 项目失败'
  }
}

async function deleteProject(p: DockerProject) {
  if (!window.confirm(`确定移除工作区记录「${p.name}」吗？（不会删除目录中的源码与配置）`)) return
  await apiRequest(`/docker/projects/${p.id}`, { method: 'DELETE' })
  await loadAll()
}

async function runProjectAction(p: DockerProject, action: string) {
  busy.value = true
  errorMsg.value = ''
  try {
    outputLog.value = await apiRequest<string>(`/docker/projects/${p.id}/action`, {
      method: 'POST',
      body: JSON.stringify({ action }),
    })
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '执行失败'
  } finally {
    busy.value = false
  }
}

async function openComposeFile(p: DockerProject) {
  const path = `${p.work_dir.replace(/\/+$/, '')}/${p.compose_file || 'docker-compose.yml'}`
  try {
    const res = await apiRequest<{ content: string }>(`/files/content?path=${encodeURIComponent(path)}`)
    editingComposePath.value = path
    editingComposeText.value = res.content
    editComposeOpen.value = true
  } catch (err: any) {
    errorMsg.value = err?.message || '读取 Compose 文件失败'
  }
}

async function saveComposeFile() {
  try {
    await apiRequest('/files/content', {
      method: 'PUT',
      body: JSON.stringify({ path: editingComposePath.value, content: editingComposeText.value }),
    })
    editComposeOpen.value = false
    outputLog.value = `已保存 ${editingComposePath.value}`
  } catch (err: any) {
    errorMsg.value = err?.message || '保存失败'
  }
}

async function containerAct(c: DockerContainer, action: string) {
  if (action === 'remove' && !window.confirm(`确定删除容器 ${c.name} 吗？`)) return
  try {
    outputLog.value = await apiRequest<string>(`/docker/containers/${c.id}/action`, {
      method: 'POST',
      body: JSON.stringify({ action }),
    })
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '容器操作失败'
  }
}

async function viewContainerLogs(c: DockerContainer) {
  try {
    logsTitle.value = `容器日志 · ${c.name} (${c.id})`
    logsText.value = await apiRequest<string>(`/docker/containers/${c.id}/logs?tail=300`)
    logsOpen.value = true
  } catch (err: any) {
    errorMsg.value = err?.message || '获取容器日志失败'
  }
}

async function removeImg(img: DockerImage) {
  if (!window.confirm(`确定删除镜像 ${img.repository}:${img.tag} 吗？`)) return
  try {
    outputLog.value = await apiRequest<string>(`/docker/images/${img.id}`, { method: 'DELETE' })
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '删除镜像失败'
  }
}

async function pruneDocker() {
  if (!window.confirm('确定清理所有已停止容器、悬空镜像和未使用构建缓存吗？')) return
  try {
    outputLog.value = await apiRequest<string>('/docker/prune', { method: 'POST' })
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '清理失败'
  }
}

async function installDockerEngine() {
  if (!window.confirm('确定通过系统包管理器一键安装 Docker 引擎与 Compose 插件并启动服务吗？（约需 15~60 秒）')) return
  installingDocker.value = true
  errorMsg.value = ''
  outputLog.value = '正在安装 Docker 引擎与 Compose，请稍候...'
  try {
    outputLog.value = await apiRequest<string>('/docker/install', { method: 'POST' })
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '安装 Docker 引擎失败'
  } finally {
    installingDocker.value = false
  }
}

onMounted(loadAll)
</script>

<template>
  <div class="space-y-4">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <Box class="h-5 w-5" style="color: var(--zm-accent)" />
          Docker 工作区
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          建项目目录 → 传源码 / 编辑 Compose 或粘贴命令部署 → 一站查看容器状态与日志
        </p>
      </div>
      <div class="flex items-center gap-2">
        <span
          v-if="status"
          class="rounded-full px-2.5 py-1 text-xs font-medium"
          :style="
            status.available
              ? { background: 'var(--zm-accent-soft)', color: 'var(--zm-accent)' }
              : status.env && !status.env.docker_nesting_supported
                ? { background: 'rgba(239,68,68,0.14)', color: 'var(--zm-danger)' }
                : { background: 'rgba(245,158,11,0.14)', color: 'var(--zm-warning)' }
          "
        >
          {{
            status.available
              ? `Docker ${status.version}${status.env?.docker_mode === 'dood' ? ' (DooD)' : status.env?.docker_mode === 'dind' ? ' (DinD)' : ''}`
              : status.env && !status.env.docker_nesting_supported
                ? `${status.env.virt_label} · 禁止套娃 Docker`
                : 'Docker 未运行/未安装'
          }}
        </span>
        <button class="zm-btn text-xs" @click="loadAll">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
        <button class="zm-btn zm-btn-primary text-xs" @click="openCreateProject">
          <Plus class="h-3.5 w-3.5" /> 新建项目工作区
        </button>
      </div>
    </div>

    <div
      v-if="status && status.env && !status.env.docker_nesting_supported"
      class="zm-card p-4 space-y-2.5 text-xs"
      style="border-color: var(--zm-danger)"
    >
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="font-semibold" style="color: var(--zm-danger)">
          检测到当前服务器为「{{ status.env.virt_label }}」，不支持再嵌套运行 Docker（无法套娃）
        </div>
        <span
          class="rounded px-2 py-0.5 text-[11px] font-mono"
          style="background: rgba(239,68,68,0.12); color: var(--zm-danger)"
        >
          virt={{ status.env.virt_type }} · mode={{ status.env.docker_mode }}
        </span>
      </div>
      <p class="leading-relaxed opacity-85 m-0">
        {{ status.env.docker_nesting_reason }}
      </p>
      <div class="flex flex-wrap gap-2 pt-0.5">
        <button class="zm-btn zm-btn-primary !py-1 text-xs" @click="router.push('/websites')">
          前往「网站管理」原生进程部署 (Rust/Go/Python)
        </button>
        <button class="zm-btn !py-1 text-xs" @click="router.push('/store')">
          前往「软件商店」原生包安装
        </button>
      </div>
    </div>

    <div
      v-else-if="status && !status.available"
      class="zm-card p-3.5 text-xs flex flex-wrap items-center justify-between gap-3"
      style="border-color: var(--zm-warning); color: var(--zm-warning)"
    >
      <span>{{ status.message }}（你仍可先管理项目工作区目录与 Compose 文件）</span>
      <button
        class="zm-btn zm-btn-primary !py-1.5 text-xs shrink-0"
        :disabled="installingDocker"
        @click="installDockerEngine"
      >
        <Download class="h-3.5 w-3.5" />
        {{ installingDocker ? '正在安装 Docker...' : '一键安装 Docker 引擎' }}
      </button>
    </div>

    <div class="flex items-center gap-2">
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': tab === 'projects' }" @click="tab = 'projects'">
        <Layers class="h-3.5 w-3.5" /> 项目工作区 ({{ projects.length }})
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': tab === 'containers' }" @click="tab = 'containers'">
        容器状态与日志 ({{ containers.length }})
      </button>
      <button class="zm-btn !py-1.5 text-xs" :class="{ 'zm-btn-primary': tab === 'images' }" @click="tab = 'images'">
        镜像与清理 ({{ images.length }})
      </button>
    </div>

    <div v-if="errorMsg" class="zm-card p-3 text-xs whitespace-pre-wrap" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Execution Output -->
    <div v-if="outputLog" class="zm-card p-3 space-y-1.5">
      <div class="flex items-center justify-between text-xs font-semibold">
        <span>最近命令输出</span>
        <button class="opacity-60 hover:opacity-100 cursor-pointer" @click="outputLog = ''">清空</button>
      </div>
      <pre
        class="p-2.5 rounded-lg text-[11px] font-mono overflow-auto max-h-48 m-0"
        style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
      >{{ outputLog }}</pre>
    </div>

    <!-- Tab 1: Projects Workspace -->
    <div v-if="tab === 'projects'" class="grid grid-cols-1 lg:grid-cols-2 gap-4">
      <div v-if="projects.length === 0" class="zm-card p-10 text-center text-xs opacity-60 lg:col-span-2">
        还没有 Docker 项目工作区，点击「新建项目工作区」创建文件夹并粘贴 docker-compose.yml 或部署命令。
      </div>
      <div v-for="p in projects" :key="p.id" class="zm-card p-4 space-y-3">
        <div class="flex items-start justify-between gap-2">
          <div class="min-w-0">
            <div class="font-semibold text-sm truncate">{{ p.name }}</div>
            <div class="text-[11px] font-mono opacity-70 truncate">{{ p.work_dir }}</div>
          </div>
          <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="deleteProject(p)">
            <Trash2 class="h-3 w-3" />
          </button>
        </div>
        <div class="rounded-lg p-2 text-[11px] font-mono truncate" style="background: var(--zm-term-bg); color: var(--zm-term-fg)">
          $ {{ p.deploy_cmd }}
        </div>
        <div class="flex flex-wrap gap-1.5">
          <button
            class="zm-btn zm-btn-primary !py-1 !px-2.5 text-xs"
            :disabled="busy || (status?.env && !status.env.docker_nesting_supported)"
            :title="status?.env && !status.env.docker_nesting_supported ? '当前容器环境不支持嵌套 Docker' : ''"
            @click="runProjectAction(p, 'deploy_cmd')"
          >
            <Play class="h-3 w-3" /> 一键部署
          </button>
          <button class="zm-btn !py-1 !px-2 text-xs" :disabled="busy || (status?.env && !status.env.docker_nesting_supported)" @click="runProjectAction(p, 'pull')">
            <Download class="h-3 w-3" /> Pull
          </button>
          <button class="zm-btn !py-1 !px-2 text-xs" :disabled="busy || (status?.env && !status.env.docker_nesting_supported)" @click="runProjectAction(p, 'restart')">
            <RotateCw class="h-3 w-3" /> 重启
          </button>
          <button class="zm-btn !py-1 !px-2 text-xs" :disabled="busy || (status?.env && !status.env.docker_nesting_supported)" @click="runProjectAction(p, 'down')">
            <Square class="h-3 w-3" /> 停止
          </button>
          <button class="zm-btn !py-1 !px-2 text-xs" @click="openComposeFile(p)">
            <Edit3 class="h-3 w-3" /> Compose
          </button>
          <button class="zm-btn !py-1 !px-2 text-xs" @click="router.push({ path: '/terminal', query: { cwd: p.work_dir } })">
            <Terminal class="h-3 w-3" /> 终端
          </button>
          <button class="zm-btn !py-1 !px-2 text-xs" @click="router.push('/files')">
            <FolderOpen class="h-3 w-3" /> 文件
          </button>
        </div>
      </div>
    </div>

    <!-- Tab 2: Containers -->
    <div v-if="tab === 'containers'" class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">容器名称</th>
              <th class="py-2.5 px-2">镜像</th>
              <th class="py-2.5 px-2">状态</th>
              <th class="py-2.5 px-2">CPU / 内存</th>
              <th class="py-2.5 px-2">端口映射</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="containers.length === 0">
              <td colspan="6" class="py-10 text-center opacity-60">暂无容器</td>
            </tr>
            <tr v-for="c in containers" :key="c.id" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
              <td class="py-2 px-3">
                <div class="font-semibold">{{ c.name }}</div>
                <div class="font-mono text-[10px] opacity-60">{{ c.id }}</div>
              </td>
              <td class="py-2 px-2 font-mono">{{ c.image }}</td>
              <td class="py-2 px-2">
                <span :class="c.state === 'running' ? 'text-emerald-400' : 'text-amber-400'">
                  {{ c.status }}
                </span>
              </td>
              <td class="py-2 px-2 font-mono">{{ c.cpu_pct }} · {{ c.mem_usage }}</td>
              <td class="py-2 px-2 font-mono max-w-[200px] truncate" :title="c.ports">{{ c.ports || '-' }}</td>
              <td class="py-2 px-3 text-right space-x-1 whitespace-nowrap">
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="viewContainerLogs(c)">
                  <ScrollText class="h-3 w-3" /> 日志
                </button>
                <button v-if="c.state !== 'running'" class="zm-btn !py-1 !px-2 text-[11px]" @click="containerAct(c, 'start')">启动</button>
                <button v-else class="zm-btn !py-1 !px-2 text-[11px]" @click="containerAct(c, 'stop')">停止</button>
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="containerAct(c, 'restart')">重启</button>
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="containerAct(c, 'remove')">删除</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Tab 3: Images & Prune -->
    <div v-if="tab === 'images'" class="space-y-3">
      <div class="flex justify-end">
        <button class="zm-btn zm-btn-danger text-xs" @click="pruneDocker">
          <Trash2 class="h-3.5 w-3.5" /> 一键清理无用资源 (system prune)
        </button>
      </div>
      <div class="zm-card overflow-hidden">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">仓库</th>
              <th class="py-2.5 px-2">标签</th>
              <th class="py-2.5 px-2">ID</th>
              <th class="py-2.5 px-2">大小</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="images.length === 0">
              <td colspan="5" class="py-10 text-center opacity-60">暂无镜像</td>
            </tr>
            <tr v-for="img in images" :key="img.id + img.tag" class="border-b" style="border-color: var(--zm-border)">
              <td class="py-2 px-3 font-mono">{{ img.repository }}</td>
              <td class="py-2 px-2 font-mono">{{ img.tag }}</td>
              <td class="py-2 px-2 font-mono opacity-70">{{ img.id }}</td>
              <td class="py-2 px-2 font-mono">{{ img.size }}</td>
              <td class="py-2 px-3 text-right">
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="removeImg(img)">删除</button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- Create Project Modal -->
    <div v-if="modalOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/65 p-4 overflow-y-auto">
      <div class="zm-card w-full max-w-2xl p-5 space-y-3 my-auto">
        <div class="flex items-center justify-between">
          <div class="font-semibold text-sm">新建 Docker 项目工作区</div>
          <button class="opacity-60 hover:opacity-100 cursor-pointer" @click="modalOpen = false"><X class="h-4 w-4" /></button>
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-2 gap-3 text-xs">
          <div>
            <label class="block mb-1 opacity-75">项目名称</label>
            <input v-model="form.name" class="zm-input" placeholder="例如：memos-blog" />
          </div>
          <div>
            <label class="block mb-1 opacity-75">工作目录（留空则自动建在数据目录）</label>
            <input v-model="form.work_dir" class="zm-input font-mono" placeholder="/opt/docker/memos" />
          </div>
          <div class="sm:col-span-2">
            <label class="block mb-1 opacity-75">部署命令（支持 docker compose 或 docker run 命令）</label>
            <input v-model="form.deploy_cmd" class="zm-input font-mono" placeholder="docker compose up -d" />
          </div>
          <div class="sm:col-span-2">
            <label class="block mb-1 opacity-75">docker-compose.yml 初始内容</label>
            <textarea v-model="form.compose_content" rows="7" class="zm-input font-mono text-xs" />
          </div>
        </div>
        <div class="flex justify-end gap-2 pt-2">
          <button class="zm-btn" @click="modalOpen = false">取消</button>
          <button class="zm-btn zm-btn-primary" @click="saveProject">创建工作区</button>
        </div>
      </div>
    </div>

    <!-- Compose CodeMirror Modal -->
    <div v-if="editComposeOpen" class="fixed inset-0 z-50 flex flex-col p-4 bg-black/70">
      <div class="zm-card flex-1 flex flex-col overflow-hidden">
        <div class="flex items-center justify-between px-4 py-2.5 border-b" style="border-color: var(--zm-border)">
          <span class="font-mono text-xs font-semibold">{{ editingComposePath }}</span>
          <div class="flex gap-2">
            <button class="zm-btn zm-btn-primary !py-1 text-xs" @click="saveComposeFile">保存 (Ctrl+S)</button>
            <button class="zm-btn !py-1 text-xs" @click="editComposeOpen = false">关闭</button>
          </div>
        </div>
        <div class="flex-1 min-h-0 p-2">
          <CodeEditor v-model="editingComposeText" filename="docker-compose.yml" :dark="theme.isDark()" @save="saveComposeFile" />
        </div>
      </div>
    </div>

    <!-- Container Logs Modal -->
    <div v-if="logsOpen" class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4">
      <div class="zm-card w-full max-w-4xl h-[75vh] flex flex-col overflow-hidden">
        <div class="flex items-center justify-between px-4 py-2.5 border-b" style="border-color: var(--zm-border)">
          <span class="text-xs font-semibold">{{ logsTitle }}</span>
          <button class="zm-btn !py-1 text-xs" @click="logsOpen = false">关闭</button>
        </div>
        <pre
          class="flex-1 overflow-auto p-4 text-xs font-mono m-0"
          style="background: var(--zm-term-bg); color: var(--zm-term-fg)"
        >{{ logsText }}</pre>
      </div>
    </div>
  </div>
</template>
