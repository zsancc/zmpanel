import { defineStore } from 'pinia'
import { ref } from 'vue'
import { apiRequest, setStoredToken } from '@/api/client'

export interface UserProfile {
  username: string
  totp_enabled: boolean
  security_entry: string
  version: string
  mode: string
}

export const useAuthStore = defineStore('auth', () => {
  const profile = ref<UserProfile | null>(null)
  const initialized = ref(false)

  async function fetchMe(): Promise<boolean> {
    try {
      const data = await apiRequest<UserProfile>('/auth/me')
      profile.value = data
      initialized.value = true
      return true
    } catch {
      profile.value = null
      initialized.value = true
      return false
    }
  }

  async function login(username: string, password: string, totpCode?: string) {
    const data = await apiRequest<{ token: string; profile: UserProfile }>('/auth/login', {
      method: 'POST',
      body: JSON.stringify({
        username,
        password,
        totp_code: totpCode || undefined,
      }),
    })
    setStoredToken(data.token)
    profile.value = data.profile
    initialized.value = true
    return data.profile
  }

  async function logout() {
    try {
      await apiRequest('/auth/logout', { method: 'POST' })
    } finally {
      setStoredToken('')
      profile.value = null
    }
  }

  return { profile, initialized, fetchMe, login, logout }
})
