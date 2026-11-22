<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { KeyRound, Lock, Palette, ShieldCheck, User } from 'lucide-vue-next'
import { useAuthStore } from '@/stores/auth'
import { THEME_PRESETS, useThemeStore } from '@/stores/theme'

const router = useRouter()
const auth = useAuthStore()
const theme = useThemeStore()

const username = ref('admin')
const password = ref('')
const totpCode = ref('')
const showTotp = ref(false)
const loading = ref(false)
const errorMsg = ref('')

async function onSubmit() {
  errorMsg.value = ''
  loading.value = true
  try {
    await auth.login(username.value.trim(), password.value, totpCode.value.trim() || undefined)
    router.replace({ name: 'dashboard' })
  } catch (err: any) {
    const msg = err?.message || '登录失败'
    errorMsg.value = msg
    if (msg.includes('2FA')) {
      showTotp.value = true
    }
  } finally {
    loading.value = false
  }
}

function cycleTheme() {
  const idx = THEME_PRESETS.findIndex((t) => t.id === theme.themeId)
  const next = THEME_PRESETS[(idx + 1) % THEME_PRESETS.length]
  theme.setTheme(next.id)
}
</script>

<template>
  <div
    class="min-h-screen flex items-center justify-center p-4 relative"
    style="background: var(--zm-bg); color: var(--zm-text)"
  >
    <button
      class="zm-btn absolute top-4 right-4 text-xs"
      @click="cycleTheme"
    >
      <Palette class="h-4 w-4" style="color: var(--zm-accent)" />
      切换主题
    </button>

    <div class="w-full max-w-md zm-card p-7 shadow-2xl">
      <div class="flex flex-col items-center text-center mb-6">
        <div
          class="flex h-12 w-12 items-center justify-center rounded-2xl text-white shadow-lg mb-3"
          style="background: var(--zm-accent)"
        >
          <ShieldCheck class="h-6 w-6" />
        </div>
        <h1 class="text-xl font-bold tracking-tight">zmpanel</h1>
        <p class="text-xs mt-1" style="color: var(--zm-text-muted)">
          轻量 · 安全 · 零外联的个人服务器运维面板
        </p>
      </div>

      <form class="space-y-4" @submit.prevent="onSubmit">
        <div>
          <label class="block text-xs font-medium mb-1.5" style="color: var(--zm-text-muted)">
            管理员账号
          </label>
          <div class="relative">
            <User class="h-4 w-4 absolute left-3 top-2.5 opacity-50" />
            <input
              v-model="username"
              type="text"
              autocomplete="username"
              required
              class="zm-input !pl-9"
              placeholder="admin"
            />
          </div>
        </div>

        <div>
          <label class="block text-xs font-medium mb-1.5" style="color: var(--zm-text-muted)">
            登录密码
          </label>
          <div class="relative">
            <Lock class="h-4 w-4 absolute left-3 top-2.5 opacity-50" />
            <input
              v-model="password"
              type="password"
              autocomplete="current-password"
              required
              class="zm-input !pl-9"
              placeholder="请输入管理员密码"
            />
          </div>
        </div>

        <div v-if="showTotp">
          <label class="block text-xs font-medium mb-1.5" style="color: var(--zm-text-muted)">
            2FA 动态验证码（6 位）
          </label>
          <div class="relative">
            <KeyRound class="h-4 w-4 absolute left-3 top-2.5 opacity-50" />
            <input
              v-model="totpCode"
              type="text"
              inputmode="numeric"
              maxlength="6"
              class="zm-input !pl-9 tracking-widest"
              placeholder="000000"
            />
          </div>
        </div>

        <div class="flex items-center justify-between text-xs">
          <button
            type="button"
            class="opacity-70 hover:opacity-100 underline cursor-pointer"
            @click="showTotp = !showTotp"
          >
            {{ showTotp ? '隐藏 2FA 验证码' : '已开启 2FA？输入动态码' }}
          </button>
        </div>

        <div
          v-if="errorMsg"
          class="rounded-lg px-3 py-2 text-xs"
          style="background: rgba(239, 68, 68, 0.12); color: var(--zm-danger); border: 1px solid rgba(239, 68, 68, 0.3)"
        >
          {{ errorMsg }}
        </div>

        <button
          type="submit"
          :disabled="loading"
          class="zm-btn zm-btn-primary w-full !py-2.5 text-sm font-semibold"
        >
          {{ loading ? '验证中...' : '安全登录' }}
        </button>
      </form>
    </div>
  </div>
</template>
