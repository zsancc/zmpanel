<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import { CheckCircle2, Layers } from 'lucide-vue-next'

interface ModuleSpec {
  title: string
  stage: string
  summary: string
  features: string[]
}

const SPECS: Record<string, ModuleSpec> = {
  websites: {
    title: '网站管理',
    stage: '阶段 2 计划交付',
    summary: '彻底去除 PHP/Java/.Net 等冗余，专注现代高频项目部署与自动进程守护。',
    features: [
      '支持 5 类项目：Rust 项目、Go 项目、Python 项目（联动 uv）、反向代理、HTML 纯静态',
      '跨系统进程守护（systemd / OpenRC / Rust 内置极简守护器兜底，无 systemd 的 NAT 鸡也能稳跑）',
      '自动生成并校验 Nginx 配置，一键绑定 SSL 证书、开启 WebSocket 与强制 HTTPS',
      '在线查看单个项目的标准输出/错误日志与一键重启',
    ],
  },
  docker: {
    title: 'Docker 工作区',
    stage: '阶段 2 计划交付',
    summary: '摒弃臃肿应用商店，围绕「建项目文件夹 → 传源码/改 Compose 或命令行部署 → 状态与日志监控」打造。',
    features: [
      '直连 /var/run/docker.sock 原生异步 API，零额外内存开销；未安装 Docker 时自动静默',
      '项目工作区：一键建目录、编辑 docker-compose.yml、粘贴命令部署并联动 Web 终端',
      '容器实时状态、CPU/内存占用、实时流式日志（支持搜索与追踪）、一键进入容器终端',
      '镜像、数据卷、网络清理与重建管理',
    ],
  },
  ssl: {
    title: 'SSL 证书中心',
    stage: '阶段 2 计划交付',
    summary: '完全独立于建站，全生命周期统一管理任意域名证书。',
    features: [
      '纯 Rust ACME v2 客户端（Let’s Encrypt / ZeroSSL），零外部脚本依赖',
      '支持 HTTP-01 与 DNS-01（Cloudflare / 阿里云 / 腾讯云 DNSPod 等），原生支持通配符证书',
      '自动后台检测续期、手动上传/导出证书与私钥',
      '支持续期后执行自定义 Hook（自动 reload Nginx 或通知其他服务）',
    ],
  },
  store: {
    title: '软件商店',
    stage: '阶段 2~3 计划交付',
    summary: '基于声明式 TOML 清单，跨 Debian / Ubuntu / Alpine 适配，支持宿主机原生与容器双模式。',
    features: [
      'Nginx 专属配置面板：主配置、Gzip/性能调优、轻量安全防护（限流/防恶意路径/IP 拦截，替代重型 WAF）',
      'Python 管理器（核心集成 uv）：多版本 Python 一键安装、创建/管理虚拟环境、包与镜像源管理',
      'Redis、PostgreSQL、MongoDB 安装、基础配置与启停（自动提示低内存/Alpine 兼容性）',
      '支持一键更新软件版本及后续自由扩展新软件清单',
    ],
  },
  firewall: {
    title: '防火墙管理',
    stage: '阶段 3 计划交付',
    summary: '统一抽象底层差异，优先原生操作 nftables 并兼容 iptables/ufw。',
    features: [
      '可视化端口放行/拦截、IP 黑白名单、端口转发规则',
      '内置防自锁保护：强制保护当前 SSH 端口与面板端口，避免误操作失联',
      '与安全中心联动，毫秒级生效封禁集合（nftables set）',
    ],
  },
  security: {
    title: '安全中心（Rust 内置轻量防御引擎）',
    stage: '阶段 3 计划交付',
    summary: '替代吃内存的 Python fail2ban，内存占用 < 2MB，专治 SSH 爆破、端口扫描与异常访问。',
    features: [
      '自动适配 Debian journald / Ubuntu auth.log / Alpine messages 的 SSH 防爆破与自动封禁',
      '基于内核防火墙日志识别端口扫描行为并自动拉黑',
      'Nginx 异常 Web 扫描（.env / wp-login / 高频 CC）自动拦截',
      'SSH 一键安全加固：改端口、禁密码仅密钥登录、禁 root 密码登录、IP 白名单与解封管理',
    ],
  },
  logs: {
    title: '日志中心',
    stage: '阶段 3 计划交付',
    summary: '一站式实时追踪与检索系统、安全、Web 及项目运行日志。',
    features: [
      'WebSocket 实时 tail -f 追踪、关键字过滤、日志下载与大小轮转清理',
      '集中聚合系统日志、SSH 拦截日志、Nginx 访问/错误日志、项目日志与计划任务日志',
    ],
  },
  cron: {
    title: '计划任务',
    stage: '阶段 3 计划交付',
    summary: '可视化 Cron 定时任务调度器，保留完整执行历史与输出。',
    features: [
      '支持 Shell 脚本、HTTP 请求、目录定时备份、日志/缓存清理、服务或容器定时重启',
      '支持手动立即试跑、查看每次执行耗时与 stdout/stderr 输出',
    ],
  },
  backup: {
    title: '备份与跨机迁移中心',
    stage: '阶段 2（本地备份）+ 阶段 4（主控仓库与跨机恢复）',
    summary: '专为经常换小鸡的场景设计：按需勾选网站或 Docker 配置，一键备份到主控鸡并在新机器一键还原。',
    features: [
      '按项自由勾选：网站项目（源码 + Nginx 配置 + 进程配置）、Docker 工作区（Compose + .env + 可选数据卷）、SSL 证书、面板配置或任意目录',
      '一键推送到主控鸡备份仓库：复用 Agent 反向加密通道传输，NAT 小鸡无需映射端口即可自动归档与按份数轮转',
      '换新小鸡一键搬家：在主控选择历史快照 → 一键下发恢复到新节点，自动校验 SHA256 并还原配置',
      '支持与计划任务联动，按日/按周定时自动备份推送至主控或本地下载',
    ],
  },
  nodes: {
    title: '多节点集群与批量更新中心',
    stage: '阶段 4 计划交付',
    summary: '单二进制 Master/Agent 架构，Agent 主动加密反连主控，免端口映射穿透 NAT 小鸡。',
    features: [
      '受控节点（Agent）无前端、常驻内存 ~10MB，适配 256MB NAT 小鸡',
      '跨节点批量下发命令与批量更新软件（如一键批量升级全部节点 Nginx 修复安全漏洞）',
      '面板与 Agent 在线一键自更新（SHA256 校验 + 原子替换 + 异常自动回滚）',
      '离线/攻击/高负载告警通知（Telegram / Webhook）',
    ],
  },
}

const route = useRoute()
const spec = computed<ModuleSpec>(() => {
  const slug = String(route.params.slug || '')
  return (
    SPECS[slug] || {
      title: '规划中模块',
      stage: '后续阶段',
      summary: '该模块已纳入 zmpanel 分阶段路线图。',
      features: [],
    }
  )
})
</script>

<template>
  <div class="max-w-3xl space-y-4">
    <div class="zm-card p-6 space-y-4">
      <div class="flex flex-wrap items-center justify-between gap-2">
        <div class="flex items-center gap-2.5">
          <div
            class="flex h-9 w-9 items-center justify-center rounded-xl text-white"
            style="background: var(--zm-accent)"
          >
            <Layers class="h-5 w-5" />
          </div>
          <div>
            <h1 class="text-lg font-bold">{{ spec.title }}</h1>
            <p class="text-xs" style="color: var(--zm-text-muted)">{{ spec.summary }}</p>
          </div>
        </div>
        <span
          class="rounded-full px-3 py-1 text-xs font-semibold"
          style="background: var(--zm-accent-soft); color: var(--zm-accent)"
        >
          {{ spec.stage }}
        </span>
      </div>

      <div class="pt-2 space-y-2.5">
        <div
          v-for="(feat, i) in spec.features"
          :key="i"
          class="flex items-start gap-2.5 rounded-lg p-3 text-xs"
          style="background: var(--zm-surface-elevated); border: 1px solid var(--zm-border)"
        >
          <CheckCircle2 class="h-4 w-4 shrink-0 mt-0.5" style="color: var(--zm-accent)" />
          <span>{{ feat }}</span>
        </div>
      </div>
    </div>
  </div>
</template>
