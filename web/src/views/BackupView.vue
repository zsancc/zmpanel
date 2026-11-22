<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { Download, HardDriveUpload, Plus, RefreshCw, RotateCcw, Server, Trash2 } from 'lucide-vue-next'
import { apiRequest, apiUrl, formatBytes, formatTimestamp, getStoredToken } from '@/api/client'

interface BackupItem {
  id: string
  name: string
  file_path: string
  size_bytes: number
  sha256: string
  target_repo: string
  scopes: string[]
  summary: string
  created_at: number
}

interface SiteBrief {
  id: string
  name: string
  kind: string
}

interface DockerBrief {
  id: string
  name: string
  work_dir: string
}

interface SslBrief {
  id: string
  name: string
}

const backups = ref<BackupItem[]>([])
const sites = ref<SiteBrief[]>([])
const dockerProjects = ref<DockerBrief[]>([])
const sslCerts = ref<SslBrief[]>([])

const selectedSites = ref<string[]>([])
const selectedDocker = ref<string[]>([])
const selectedSsl = ref<string[]>([])
const customDirsText = ref('')
const includePanelDb = ref(true)
const backupName = ref('')
const targetRepo = ref<'local' | 'master'>('local')

const busy = ref(false)
const notice = ref('')
const errorMsg = ref('')

async function loadAll() {
  try {
    const [b, w, d, s] = await Promise.all([
      apiRequest<BackupItem[]>('/backups'),
      apiRequest<SiteBrief[]>('/websites').catch(() => []),
      apiRequest<DockerBrief[]>('/docker/projects').catch(() => []),
      apiRequest<SslBrief[]>('/ssl/certs').catch(() => []),
    ])
    backups.value = b
    sites.value = w
    dockerProjects.value = d
    sslCerts.value = s
  } catch (err: any) {
    errorMsg.value = err?.message || '加载备份数据失败'
  }
}

async function createBackup() {
  busy.value = true
  errorMsg.value = ''
  notice.value = ''
  const custom_dirs = customDirsText.value
    .split('\n')
    .map((s) => s.trim())
    .filter(Boolean)
  try {
    const created = await apiRequest<BackupItem>('/backups', {
      method: 'POST',
      body: JSON.stringify({
        name: backupName.value.trim() || undefined,
        website_ids: selectedSites.value,
        docker_project_ids: selectedDocker.value,
        ssl_cert_ids: selectedSsl.value,
        custom_dirs,
        include_panel_db: includePanelDb.value,
        target_repo: targetRepo.value,
      }),
    })
    backupName.value = ''
    notice.value = `备份创建成功：${created.name} (${formatBytes(created.size_bytes)}, SHA256: ${created.sha256.slice(0, 12)}...)`
    await loadAll()
  } catch (err: any) {
    errorMsg.value = err?.message || '创建备份失败'
  } finally {
    busy.value = false
  }
}

async function restoreBackup(b: BackupItem) {
  const dir = window.prompt(
    `确定校验 SHA256 并恢复备份「${b.name}」吗？\n可指定解压目标目录（留空则安全恢复至默认还原目录）：`,
    '',
  )
  if (dir === null) return
  busy.value = true
  errorMsg.value = ''
  try {
    const msg = await apiRequest<string>(`/backups/${b.id}/restore`, {
      method: 'POST',
      body: JSON.stringify({ restore_dir: dir.trim() || undefined }),
    })
    notice.value = msg
  } catch (err: any) {
    errorMsg.value = err?.message || '恢复失败'
  } finally {
    busy.value = false
  }
}

function downloadBackup(b: BackupItem) {
  const token = getStoredToken()
  fetch(apiUrl(`/backups/${b.id}/download`), {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    credentials: 'include',
  })
    .then((r) => {
      if (!r.ok) throw new Error('下载失败')
      return r.blob()
    })
    .then((blob) => {
      const a = document.createElement('a')
      a.href = URL.createObjectURL(blob)
      a.download = `${b.name}.tar.gz`
      a.click()
      URL.revokeObjectURL(a.href)
    })
    .catch((err) => {
      errorMsg.value = err?.message || '下载失败'
    })
}

async function deleteBackup(b: BackupItem) {
  if (!window.confirm(`确定删除备份快照「${b.name}」吗？`)) return
  await apiRequest(`/backups/${b.id}`, { method: 'DELETE' })
  await loadAll()
}

onMounted(loadAll)
</script>

<template>
  <div class="space-y-5">
    <div class="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 class="text-lg font-bold flex items-center gap-2">
          <HardDriveUpload class="h-5 w-5" style="color: var(--zm-accent)" />
          备份与跨机迁移
        </h1>
        <p class="text-xs mt-0.5" style="color: var(--zm-text-muted)">
          自由勾选网站 / Docker 配置 / SSL 证书 / 目录一键打包（含 manifest.json 与 SHA256 校验，阶段 4 接通主控后支持 NAT 鸡一键推送至主控仓库）
        </p>
      </div>
      <button class="zm-btn text-xs" @click="loadAll">
        <RefreshCw class="h-3.5 w-3.5" /> 刷新
      </button>
    </div>

    <div v-if="notice" class="zm-card p-3 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card p-3 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- Backup Builder -->
    <div class="zm-card p-5 space-y-4 text-xs">
      <div class="font-semibold text-sm">创建新备份快照（按需勾选）</div>

      <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
        <!-- Websites -->
        <div class="rounded-lg p-3 space-y-2" style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)">
          <div class="font-semibold">网站项目 ({{ sites.length }})</div>
          <div v-if="sites.length === 0" class="opacity-60">暂无网站</div>
          <label v-for="w in sites" :key="w.id" class="flex items-center gap-2 cursor-pointer">
            <input v-model="selectedSites" type="checkbox" :value="w.id" />
            <span class="truncate">{{ w.name }} ({{ w.kind }})</span>
          </label>
        </div>

        <!-- Docker Projects -->
        <div class="rounded-lg p-3 space-y-2" style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)">
          <div class="font-semibold">Docker 工作区 ({{ dockerProjects.length }})</div>
          <div v-if="dockerProjects.length === 0" class="opacity-60">暂无 Docker 工作区</div>
          <label v-for="d in dockerProjects" :key="d.id" class="flex items-center gap-2 cursor-pointer">
            <input v-model="selectedDocker" type="checkbox" :value="d.id" />
            <span class="truncate">{{ d.name }}</span>
          </label>
        </div>

        <!-- SSL & Panel DB -->
        <div class="rounded-lg p-3 space-y-2" style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)">
          <div class="font-semibold">SSL 证书与面板配置</div>
          <label class="flex items-center gap-2 cursor-pointer font-medium">
            <input v-model="includePanelDb" type="checkbox" />
            包含面板数据库 (zmpanel.db)
          </label>
          <div class="border-t pt-2 space-y-1.5" style="border-color: var(--zm-border)">
            <div v-if="sslCerts.length === 0" class="opacity-60">暂无 SSL 证书</div>
            <label v-for="c in sslCerts" :key="c.id" class="flex items-center gap-2 cursor-pointer">
              <input v-model="selectedSsl" type="checkbox" :value="c.id" />
              <span class="truncate">SSL: {{ c.name }}</span>
            </label>
          </div>
        </div>
      </div>

      <div class="grid grid-cols-1 md:grid-cols-3 gap-3">
        <div>
          <label class="block mb-1 opacity-75">快照名称（留空自动带时间戳）</label>
          <input v-model="backupName" class="zm-input" placeholder="例如：before-migrate-vps" />
        </div>
        <div>
          <label class="block mb-1 opacity-75">自定义追加目录（每行一个路径）</label>
          <textarea v-model="customDirsText" rows="1" class="zm-input font-mono" placeholder="/etc/nginx/conf.d" />
        </div>
        <div>
          <label class="block mb-1 opacity-75">目标存储仓库</label>
          <select v-model="targetRepo" class="zm-input">
            <option value="local">本机备份仓库（可随时下载/还原）</option>
          </select>
          <div class="text-[11px] opacity-65 mt-1">
            多台 NAT 小鸡统一备份至主控仓库与跨机恢复，请前往「节点集群 → 主控中央备份仓库」。
          </div>
        </div>
      </div>

      <div class="flex items-center justify-between pt-1">
        <span class="opacity-65 flex items-center gap-1.5">
          <Server class="h-3.5 w-3.5" /> 归档格式兼容阶段 4 跨机搬家，换新小鸡可直接导入恢复
        </span>
        <button class="zm-btn zm-btn-primary text-xs" :disabled="busy" @click="createBackup">
          <Plus class="h-3.5 w-3.5" /> {{ busy ? '打包中...' : '一键打包备份' }}
        </button>
      </div>
    </div>

    <!-- Backup List -->
    <div class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3">快照名称</th>
              <th class="py-2.5 px-2">备份内容</th>
              <th class="py-2.5 px-2">大小 / SHA256</th>
              <th class="py-2.5 px-2">创建时间</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="backups.length === 0">
              <td colspan="5" class="py-10 text-center opacity-60">暂无备份记录</td>
            </tr>
            <tr v-for="b in backups" :key="b.id" class="border-b hover:bg-white/5" style="border-color: var(--zm-border)">
              <td class="py-2.5 px-3 font-semibold">{{ b.name }}</td>
              <td class="py-2.5 px-2">{{ b.summary }}</td>
              <td class="py-2.5 px-2 font-mono">
                <div>{{ formatBytes(b.size_bytes) }}</div>
                <div class="text-[10px] opacity-60">{{ b.sha256.slice(0, 16) }}...</div>
              </td>
              <td class="py-2.5 px-2 opacity-75">{{ formatTimestamp(b.created_at) }}</td>
              <td class="py-2.5 px-3 text-right space-x-1 whitespace-nowrap">
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="downloadBackup(b)">
                  <Download class="h-3 w-3" /> 下载
                </button>
                <button class="zm-btn !py-1 !px-2 text-[11px]" :disabled="busy" @click="restoreBackup(b)">
                  <RotateCcw class="h-3 w-3" /> 恢复
                </button>
                <button class="zm-btn zm-btn-danger !py-1 !px-2 text-[11px]" @click="deleteBackup(b)">
                  <Trash2 class="h-3 w-3" />
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>
  </div>
</template>
