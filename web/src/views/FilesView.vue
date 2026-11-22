<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import {
  Archive,
  Check,
  Copy,
  Download,
  Edit3,
  File,
  FilePlus,
  Folder,
  FolderPlus,
  RefreshCw,
  RotateCcw,
  Save,
  Terminal,
  Trash2,
  Upload,
  X,
} from 'lucide-vue-next'
import CodeEditor from '@/components/CodeEditor.vue'
import { apiRequest, apiUrl, formatBytes, formatTimestamp, getStoredToken } from '@/api/client'
import { useThemeStore } from '@/stores/theme'

interface FileEntry {
  name: string
  path: string
  is_dir: boolean
  is_symlink: boolean
  size: number
  modified: number
  permissions: string
  mode_octal: string
}

interface TrashItem {
  id: string
  original_path: string
  name: string
  is_dir: boolean
  size: number
  deleted_at: number
}

const router = useRouter()
const theme = useThemeStore()

const currentPath = ref('')
const pathInput = ref('')
const entries = ref<FileEntry[]>([])
const selectedPaths = ref<string[]>([])
const loading = ref(false)
const notice = ref('')
const errorMsg = ref('')

// Editor modal
const editorOpen = ref(false)
const editingPath = ref('')
const editingContent = ref('')
const editingOriginal = ref('')
const editorSaving = ref(false)
const copySuccess = ref(false)

// Trash drawer
const trashOpen = ref(false)
const trashItems = ref<TrashItem[]>([])

const fileUploadInput = ref<HTMLInputElement | null>(null)

const isDirty = computed(() => editingContent.value !== editingOriginal.value)

function joinPath(base: string, name: string): string {
  const cleanBase = base.replace(/\/+$/, '')
  if (!cleanBase) return `/${name}`
  return `${cleanBase}/${name}`
}

function parentPath(path: string): string {
  const normalized = path.replace(/\\/g, '/').replace(/\/+$/, '')
  if (!normalized || normalized === '/') return '/'
  const idx = normalized.lastIndexOf('/')
  if (idx <= 0) {
    // Windows drive root like "E:"
    if (/^[A-Za-z]:$/.test(normalized)) return `${normalized}/`
    return '/'
  }
  const parent = normalized.slice(0, idx)
  return /^[A-Za-z]:$/.test(parent) ? `${parent}/` : parent
}

function showNotice(msg: string) {
  notice.value = msg
  errorMsg.value = ''
  setTimeout(() => {
    if (notice.value === msg) notice.value = ''
  }, 3500)
}

async function loadDir(targetPath = currentPath.value) {
  loading.value = true
  errorMsg.value = ''
  selectedPaths.value = []
  try {
    const query = targetPath ? `?path=${encodeURIComponent(targetPath)}` : ''
    const data = await apiRequest<{ path: string; entries: FileEntry[] }>(`/files/list${query}`)
    currentPath.value = data.path
    pathInput.value = data.path
    entries.value = data.entries
  } catch (err: any) {
    errorMsg.value = err?.message || '读取目录失败'
  } finally {
    loading.value = false
  }
}

async function openEntry(entry: FileEntry) {
  if (entry.is_dir) {
    await loadDir(entry.path)
  } else {
    await openEditor(entry.path)
  }
}

async function openEditor(path: string) {
  try {
    const data = await apiRequest<{ path: string; size: number; content: string }>(
      `/files/content?path=${encodeURIComponent(path)}`,
    )
    editingPath.value = data.path
    editingContent.value = data.content
    editingOriginal.value = data.content
    editorOpen.value = true
  } catch (err: any) {
    errorMsg.value = err?.message || '打开文件失败'
  }
}

async function saveEditor() {
  editorSaving.value = true
  try {
    await apiRequest('/files/content', {
      method: 'PUT',
      body: JSON.stringify({ path: editingPath.value, content: editingContent.value }),
    })
    editingOriginal.value = editingContent.value
    showNotice(`已保存 ${editingPath.value}`)
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '保存文件失败'
  } finally {
    editorSaving.value = false
  }
}

async function copyAllEditorContent() {
  try {
    await navigator.clipboard.writeText(editingContent.value)
    copySuccess.value = true
    setTimeout(() => (copySuccess.value = false), 1800)
  } catch {
    errorMsg.value = '浏览器剪贴板写入失败，请使用编辑器内 Ctrl+A / Ctrl+C'
  }
}

async function createItem(isDir: boolean) {
  const label = isDir ? '文件夹' : '文件'
  const name = window.prompt(`新建${label}名称：`)
  if (!name || !name.trim()) return
  try {
    await apiRequest('/files/create', {
      method: 'POST',
      body: JSON.stringify({ path: joinPath(currentPath.value, name.trim()), is_dir: isDir }),
    })
    showNotice(`已创建${label}：${name.trim()}`)
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || `创建${label}失败`
  }
}

async function renameItem(entry: FileEntry) {
  const newName = window.prompt('输入新名称（或完整目标路径）：', entry.name)
  if (!newName || newName.trim() === entry.name) return
  const target = newName.includes('/') || newName.includes('\\')
    ? newName.trim()
    : joinPath(currentPath.value, newName.trim())
  try {
    await apiRequest('/files/rename', {
      method: 'POST',
      body: JSON.stringify({ from: entry.path, to: target }),
    })
    showNotice('重命名/移动成功')
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '重命名失败'
  }
}

async function deleteSelected(permanent = false) {
  if (selectedPaths.value.length === 0) return
  const msg = permanent
    ? `确定彻底删除选中的 ${selectedPaths.value.length} 项吗？此操作不可恢复！`
    : `确定将选中的 ${selectedPaths.value.length} 项移入回收站吗？`
  if (!window.confirm(msg)) return
  try {
    await apiRequest('/files/delete', {
      method: 'POST',
      body: JSON.stringify({ paths: selectedPaths.value, permanent }),
    })
    showNotice(permanent ? '已彻底删除' : '已移入回收站')
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '删除失败'
  }
}

async function compressSelected() {
  if (selectedPaths.value.length === 0) return
  const name = window.prompt('输入压缩包名称（支持 .zip 或 .tar.gz）：', 'archive.tar.gz')
  if (!name || !name.trim()) return
  try {
    const dest = joinPath(currentPath.value, name.trim())
    await apiRequest('/files/compress', {
      method: 'POST',
      body: JSON.stringify({ paths: selectedPaths.value, dest }),
    })
    showNotice(`压缩成功：${name.trim()}`)
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '压缩失败'
  }
}

async function extractItem(entry: FileEntry) {
  const dest = window.prompt('解压到目录：', currentPath.value)
  if (!dest || !dest.trim()) return
  try {
    await apiRequest('/files/extract', {
      method: 'POST',
      body: JSON.stringify({ archive: entry.path, dest_dir: dest.trim() }),
    })
    showNotice('解压完成')
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '解压失败'
  }
}

async function chmodItem(entry: FileEntry) {
  const mode = window.prompt('输入 POSIX 八进制权限（例如 755 或 644）：', entry.mode_octal)
  if (!mode || !mode.trim()) return
  try {
    await apiRequest('/files/chmod', {
      method: 'POST',
      body: JSON.stringify({ path: entry.path, mode: mode.trim() }),
    })
    showNotice(`权限已修改为 ${mode.trim()}`)
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '修改权限失败'
  }
}

async function handleUpload(ev: Event) {
  const input = ev.target as HTMLInputElement
  if (!input.files || input.files.length === 0) return
  const form = new FormData()
  for (const f of Array.from(input.files)) {
    form.append('file', f)
  }
  try {
    await apiRequest(`/files/upload?path=${encodeURIComponent(currentPath.value)}`, {
      method: 'POST',
      body: form,
    })
    showNotice(`成功上传 ${input.files.length} 个文件`)
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '上传失败'
  } finally {
    input.value = ''
  }
}

function downloadItem(entry: FileEntry) {
  const token = getStoredToken()
  const url = `${apiUrl('/files/download')}?path=${encodeURIComponent(entry.path)}`
  // Use authenticated fetch -> blob to support Bearer auth cleanly
  fetch(url, {
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
      a.download = entry.name
      a.click()
      URL.revokeObjectURL(a.href)
    })
    .catch((err) => {
      errorMsg.value = err?.message || '下载失败'
    })
}

async function openTrash() {
  trashOpen.value = true
  try {
    trashItems.value = await apiRequest<TrashItem[]>('/files/trash')
  } catch (err: any) {
    errorMsg.value = err?.message || '加载回收站失败'
  }
}

async function restoreTrash(item: TrashItem) {
  try {
    await apiRequest('/files/trash/restore', {
      method: 'POST',
      body: JSON.stringify({ id: item.id }),
    })
    showNotice(`已还原：${item.original_path}`)
    await openTrash()
    await loadDir()
  } catch (err: any) {
    errorMsg.value = err?.message || '还原失败'
  }
}

async function emptyTrashAll() {
  if (!window.confirm('确定彻底清空回收站吗？')) return
  try {
    await apiRequest('/files/trash', { method: 'DELETE' })
    trashItems.value = []
    showNotice('回收站已清空')
  } catch (err: any) {
    errorMsg.value = err?.message || '清空失败'
  }
}

function toggleSelectAll(ev: Event) {
  const checked = (ev.target as HTMLInputElement).checked
  selectedPaths.value = checked ? entries.value.map((e) => e.path) : []
}

function openTerminalHere() {
  router.push({ path: '/terminal', query: { cwd: currentPath.value } })
}

function isArchiveName(name: string): boolean {
  const l = name.toLowerCase()
  return l.endsWith('.zip') || l.endsWith('.tar.gz') || l.endsWith('.tgz')
}

onMounted(() => loadDir())
</script>

<template>
  <div class="space-y-4">
    <!-- Toolbar -->
    <div class="zm-card p-3.5 space-y-3">
      <div class="flex flex-wrap items-center gap-2">
        <button class="zm-btn !py-1.5 text-xs" @click="loadDir(parentPath(currentPath))">
          上级目录
        </button>
        <form class="flex-1 flex gap-2 min-w-[220px]" @submit.prevent="loadDir(pathInput)">
          <input v-model="pathInput" class="zm-input !py-1.5 font-mono text-xs" placeholder="输入目录路径回车跳转" />
          <button type="submit" class="zm-btn !py-1.5 text-xs shrink-0">前往</button>
        </form>
        <button class="zm-btn !py-1.5 text-xs" @click="loadDir()">
          <RefreshCw class="h-3.5 w-3.5" /> 刷新
        </button>
        <button class="zm-btn !py-1.5 text-xs" @click="openTerminalHere">
          <Terminal class="h-3.5 w-3.5" /> 在此打开终端
        </button>
      </div>

      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex flex-wrap items-center gap-2">
          <button class="zm-btn zm-btn-primary !py-1.5 text-xs" @click="fileUploadInput?.click()">
            <Upload class="h-3.5 w-3.5" /> 上传文件
          </button>
          <input ref="fileUploadInput" type="file" multiple class="hidden" @change="handleUpload" />
          <button class="zm-btn !py-1.5 text-xs" @click="createItem(false)">
            <FilePlus class="h-3.5 w-3.5" /> 新建文件
          </button>
          <button class="zm-btn !py-1.5 text-xs" @click="createItem(true)">
            <FolderPlus class="h-3.5 w-3.5" /> 新建目录
          </button>
          <button
            class="zm-btn !py-1.5 text-xs"
            :disabled="selectedPaths.length === 0"
            @click="compressSelected"
          >
            <Archive class="h-3.5 w-3.5" /> 压缩 ({{ selectedPaths.length }})
          </button>
          <button
            class="zm-btn zm-btn-danger !py-1.5 text-xs"
            :disabled="selectedPaths.length === 0"
            @click="deleteSelected(false)"
          >
            <Trash2 class="h-3.5 w-3.5" /> 移入回收站
          </button>
        </div>

        <button class="zm-btn !py-1.5 text-xs" @click="openTrash">
          <Trash2 class="h-3.5 w-3.5" /> 回收站
        </button>
      </div>
    </div>

    <div v-if="notice" class="zm-card px-3.5 py-2 text-xs" style="border-color: var(--zm-accent); color: var(--zm-accent)">
      {{ notice }}
    </div>
    <div v-if="errorMsg" class="zm-card px-3.5 py-2 text-xs" style="border-color: var(--zm-danger); color: var(--zm-danger)">
      {{ errorMsg }}
    </div>

    <!-- File Table -->
    <div class="zm-card overflow-hidden">
      <div class="overflow-x-auto">
        <table class="w-full text-xs text-left border-collapse">
          <thead>
            <tr class="border-b" style="border-color: var(--zm-border); background: var(--zm-surface-elevated); color: var(--zm-text-muted)">
              <th class="py-2.5 px-3 w-8">
                <input
                  type="checkbox"
                  :checked="entries.length > 0 && selectedPaths.length === entries.length"
                  @change="toggleSelectAll"
                />
              </th>
              <th class="py-2.5 px-2">名称</th>
              <th class="py-2.5 px-2">大小</th>
              <th class="py-2.5 px-2">权限</th>
              <th class="py-2.5 px-2">修改时间</th>
              <th class="py-2.5 px-3 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="loading">
              <td colspan="6" class="py-10 text-center opacity-60">正在加载目录...</td>
            </tr>
            <tr v-else-if="entries.length === 0">
              <td colspan="6" class="py-10 text-center opacity-60">当前目录为空</td>
            </tr>
            <tr
              v-for="item in entries"
              :key="item.path"
              class="border-b hover:bg-white/5 transition-colors"
              style="border-color: var(--zm-border)"
            >
              <td class="py-2 px-3">
                <input v-model="selectedPaths" type="checkbox" :value="item.path" />
              </td>
              <td class="py-2 px-2">
                <button
                  class="flex items-center gap-2 font-medium hover:underline cursor-pointer text-left"
                  @click="openEntry(item)"
                >
                  <Folder v-if="item.is_dir" class="h-4 w-4 shrink-0 text-amber-400" />
                  <File v-else class="h-4 w-4 shrink-0 opacity-70" />
                  <span>{{ item.name }}</span>
                </button>
              </td>
              <td class="py-2 px-2 font-mono opacity-80">
                {{ item.is_dir ? '-' : formatBytes(item.size) }}
              </td>
              <td class="py-2 px-2 font-mono">
                <button
                  class="hover:underline cursor-pointer opacity-80"
                  title="点击修改权限"
                  @click="chmodItem(item)"
                >
                  {{ item.mode_octal }} ({{ item.permissions }})
                </button>
              </td>
              <td class="py-2 px-2 opacity-80">{{ formatTimestamp(item.modified) }}</td>
              <td class="py-2 px-3 text-right space-x-1.5 whitespace-nowrap">
                <button
                  v-if="!item.is_dir"
                  class="zm-btn !py-1 !px-2 text-[11px]"
                  @click="openEditor(item.path)"
                >
                  <Edit3 class="h-3 w-3" /> 编辑
                </button>
                <button
                  v-if="!item.is_dir && isArchiveName(item.name)"
                  class="zm-btn !py-1 !px-2 text-[11px]"
                  @click="extractItem(item)"
                >
                  解压
                </button>
                <button
                  v-if="!item.is_dir"
                  class="zm-btn !py-1 !px-2 text-[11px]"
                  @click="downloadItem(item)"
                >
                  <Download class="h-3 w-3" />
                </button>
                <button class="zm-btn !py-1 !px-2 text-[11px]" @click="renameItem(item)">
                  重命名
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </div>

    <!-- CodeMirror 6 Editor Modal -->
    <div
      v-if="editorOpen"
      class="fixed inset-0 z-50 flex flex-col p-3 sm:p-5 bg-black/70"
    >
      <div class="zm-card flex-1 flex flex-col overflow-hidden shadow-2xl">
        <div
          class="flex flex-wrap items-center justify-between gap-2 px-4 py-2.5 border-b"
          style="border-color: var(--zm-border); background: var(--zm-surface-elevated)"
        >
          <div class="flex items-center gap-2 min-w-0">
            <span class="font-mono text-xs font-semibold truncate">{{ editingPath }}</span>
            <span
              v-if="isDirty"
              class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
              style="background: rgba(245, 158, 11, 0.16); color: var(--zm-warning)"
            >
              未保存
            </span>
          </div>
          <div class="flex items-center gap-2">
            <button class="zm-btn !py-1 !px-2.5 text-xs" @click="copyAllEditorContent">
              <Check v-if="copySuccess" class="h-3.5 w-3.5 text-emerald-400" />
              <Copy v-else class="h-3.5 w-3.5" />
              {{ copySuccess ? '已完整复制全文' : '完整复制全文' }}
            </button>
            <button
              class="zm-btn zm-btn-primary !py-1 !px-3 text-xs"
              :disabled="editorSaving"
              @click="saveEditor"
            >
              <Save class="h-3.5 w-3.5" />
              {{ editorSaving ? '保存中...' : '保存 (Ctrl+S)' }}
            </button>
            <button class="zm-btn !py-1 !px-2 text-xs" @click="editorOpen = false">
              <X class="h-3.5 w-3.5" />
            </button>
          </div>
        </div>
        <div class="flex-1 min-h-0 p-2">
          <CodeEditor
            v-model="editingContent"
            :filename="editingPath"
            :dark="theme.isDark()"
            @save="saveEditor"
          />
        </div>
        <div
          class="px-4 py-1.5 text-[11px] flex items-center justify-between border-t"
          style="border-color: var(--zm-border); color: var(--zm-text-muted)"
        >
          <span>CodeMirror 6 内核 · 支持 Ctrl+F 查找替换 · Ctrl+A / Ctrl+C 全文无损复制</span>
          <span>长度: {{ editingContent.length }} 字符</span>
        </div>
      </div>
    </div>

    <!-- Trash Drawer -->
    <div
      v-if="trashOpen"
      class="fixed inset-0 z-50 flex justify-end bg-black/60"
      @click.self="trashOpen = false"
    >
      <div class="zm-card w-full max-w-lg h-full !rounded-none flex flex-col p-4 space-y-3">
        <div class="flex items-center justify-between">
          <div class="font-semibold text-sm">回收站 ({{ trashItems.length }})</div>
          <div class="flex gap-2">
            <button class="zm-btn zm-btn-danger !py-1 text-xs" @click="emptyTrashAll">
              清空回收站
            </button>
            <button class="zm-btn !py-1 text-xs" @click="trashOpen = false">关闭</button>
          </div>
        </div>
        <div class="flex-1 overflow-y-auto space-y-2">
          <div v-if="trashItems.length === 0" class="py-10 text-center text-xs opacity-60">
            回收站为空
          </div>
          <div
            v-for="item in trashItems"
            :key="item.id"
            class="rounded-lg p-3 flex items-center justify-between gap-2"
            style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
          >
            <div class="min-w-0">
              <div class="text-xs font-semibold truncate">{{ item.name }}</div>
              <div class="text-[11px] font-mono truncate opacity-70">{{ item.original_path }}</div>
              <div class="text-[10px] opacity-50 mt-0.5">{{ formatTimestamp(item.deleted_at) }}</div>
            </div>
            <button class="zm-btn !py-1 !px-2 text-xs shrink-0" @click="restoreTrash(item)">
              <RotateCcw class="h-3.5 w-3.5" /> 还原
            </button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
