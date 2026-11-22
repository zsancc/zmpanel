import { defineStore } from 'pinia'
import { ref } from 'vue'

export interface ThemePreset {
  id: string
  name: string
  desc: string
  dark: boolean
  previewBg: string
  previewSurface: string
  previewSidebar: string
}

export interface AccentPreset {
  id: string
  name: string
  color: string
  soft: string
}

export const THEME_PRESETS: ThemePreset[] = [
  {
    id: 'light',
    name: '极简亮白',
    desc: '清爽克制，适合日间运维',
    dark: false,
    previewBg: '#f4f6f9',
    previewSurface: '#ffffff',
    previewSidebar: '#0f172a',
  },
  {
    id: 'dark',
    name: '深邃暗黑',
    desc: '黑曜石护眼暗色',
    dark: true,
    previewBg: '#0b0f17',
    previewSurface: '#131926',
    previewSidebar: '#090d14',
  },
  {
    id: 'aurora',
    name: '极光蓝调',
    desc: '现代深海冷蓝质感',
    dark: true,
    previewBg: '#0d1321',
    previewSurface: '#151e32',
    previewSidebar: '#090e1a',
  },
  {
    id: 'forest',
    name: '护眼墨绿',
    desc: '低饱和深林翠绿',
    dark: true,
    previewBg: '#0c1713',
    previewSurface: '#12221c',
    previewSidebar: '#08110e',
  },
  {
    id: 'paper',
    name: '暖灰纸墨',
    desc: '柔和纸感暖米灰',
    dark: false,
    previewBg: '#f5f2eb',
    previewSurface: '#fffdf8',
    previewSidebar: '#26231e',
  },
  {
    id: 'matrix',
    name: '终端极客',
    desc: '高对比暗黑极客绿',
    dark: true,
    previewBg: '#050806',
    previewSurface: '#0b110d',
    previewSidebar: '#030604',
  },
]

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: 'emerald', name: '翡翠绿', color: '#10b981', soft: 'rgba(16, 185, 129, 0.14)' },
  { id: 'blue', name: '科技蓝', color: '#3b82f6', soft: 'rgba(59, 130, 246, 0.14)' },
  { id: 'violet', name: '星云紫', color: '#8b5cf6', soft: 'rgba(139, 92, 246, 0.14)' },
  { id: 'amber', name: '琥珀橙', color: '#f59e0b', soft: 'rgba(245, 158, 11, 0.14)' },
  { id: 'rose', name: '玫瑰红', color: '#f43f5e', soft: 'rgba(244, 63, 94, 0.14)' },
  { id: 'cyan', name: '青瓷蓝', color: '#06b6d4', soft: 'rgba(6, 182, 212, 0.14)' },
]

export const useThemeStore = defineStore('theme', () => {
  const themeId = ref<string>(localStorage.getItem('zmpanel_theme') || 'dark')
  const accentId = ref<string>(localStorage.getItem('zmpanel_accent') || 'emerald')

  function apply() {
    document.documentElement.setAttribute('data-theme', themeId.value)
    const accent = ACCENT_PRESETS.find((a) => a.id === accentId.value) || ACCENT_PRESETS[0]
    document.documentElement.style.setProperty('--zm-accent', accent.color)
    document.documentElement.style.setProperty('--zm-accent-soft', accent.soft)
  }

  function setTheme(id: string) {
    themeId.value = id
    localStorage.setItem('zmpanel_theme', id)
    apply()
  }

  function setAccent(id: string) {
    accentId.value = id
    localStorage.setItem('zmpanel_accent', id)
    apply()
  }

  function isDark(): boolean {
    const preset = THEME_PRESETS.find((t) => t.id === themeId.value)
    return preset ? preset.dark : true
  }

  return { themeId, accentId, apply, setTheme, setAccent, isDark }
})
