declare global {
  interface Window {
    __ZMPANEL_BASE__?: string
  }
}

export interface ApiEnvelope<T> {
  ok: boolean
  data: T | null
  error: { code: number; message: string } | null
}

const TOKEN_STORAGE_KEY = 'zmpanel_token'

export function getBasePath(): string {
  const injected = window.__ZMPANEL_BASE__
  if (typeof injected === 'string' && injected.length > 0) {
    return injected.replace(/\/+$/, '')
  }
  return ''
}

export function getStoredToken(): string {
  return localStorage.getItem(TOKEN_STORAGE_KEY) || ''
}

export function setStoredToken(token: string) {
  if (token) {
    localStorage.setItem(TOKEN_STORAGE_KEY, token)
  } else {
    localStorage.removeItem(TOKEN_STORAGE_KEY)
  }
}

export function apiUrl(path: string): string {
  const clean = path.startsWith('/') ? path : `/${path}`
  return `${getBasePath()}/api${clean}`
}

export function wsUrl(path: string): string {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${proto}//${window.location.host}${apiUrl(path)}`
}

export async function apiRequest<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const headers = new Headers(options.headers || {})
  const token = getStoredToken()
  if (token && !headers.has('Authorization')) {
    headers.set('Authorization', `Bearer ${token}`)
  }
  if (!(options.body instanceof FormData) && options.body && !headers.has('Content-Type')) {
    headers.set('Content-Type', 'application/json')
  }

  const resp = await fetch(apiUrl(path), {
    ...options,
    headers,
    credentials: 'include',
  })

  const json = (await resp.json().catch(() => null)) as ApiEnvelope<T> | null
  if (!resp.ok || !json || !json.ok) {
    const msg = json?.error?.message || `请求失败 (HTTP ${resp.status})`
    if (resp.status === 401) {
      setStoredToken('')
    }
    throw new Error(msg)
  }
  return json.data as T
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let idx = 0
  let v = bytes
  while (v >= 1024 && idx < units.length - 1) {
    v /= 1024
    idx++
  }
  return `${v.toFixed(idx === 0 ? 0 : 2)} ${units[idx]}`
}

export function formatDuration(secs: number): string {
  const d = Math.floor(secs / 86400)
  const h = Math.floor((secs % 86400) / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (d > 0) return `${d} 天 ${h} 小时 ${m} 分`
  if (h > 0) return `${h} 小时 ${m} 分`
  return `${m} 分钟`
}

export function formatTimestamp(ts: number): string {
  if (!ts) return '-'
  const d = new Date(ts * 1000)
  return d.toLocaleString('zh-CN', { hour12: false })
}
