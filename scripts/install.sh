#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# zmpanel 一键安装 / 运维 / 卸载脚本 (Debian / Ubuntu / Alpine Linux)
# 仓库: https://github.com/zsancc/zmpanel
#
# 一键安装:
#   单机模式 : curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash
#   主控模式 : curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --mode master --port 18080
#   NAT Agent: curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --mode agent --master-url "wss://..." --node-id "node_xxx" --node-token "xxx"
#   国内加速 : GH_PROXY="https://ghfast.top/" curl -fsSL ... | sudo -E bash
#
# 卸载方式:
#   交互菜单 : sudo zmpanel-ctl uninstall  (或 sudo bash install.sh --uninstall)
#   仅卸面板 : sudo zmpanel-ctl uninstall --uninstall-panel --yes
#   清理面板 : sudo zmpanel-ctl uninstall --purge-panel --yes
#   完全卸载 : sudo zmpanel-ctl uninstall --uninstall-all --yes
#   指定服务 : sudo zmpanel-ctl uninstall --uninstall-all --remove-apps nginx,redis,docker --yes
# ==============================================================================

REPO="${ZMPANEL_REPO:-zsancc/zmpanel}"
GH_PROXY="${GH_PROXY:-}"
VERSION="${ZMPANEL_VERSION:-latest}"

MODE="standalone"
PORT="18080"
HOST="0.0.0.0"
DATA_DIR="/opt/zmpanel/data"
ENTRY=""
ADMIN_PASS=""
BIN_SRC=""
FROM_SOURCE=0
MASTER_URL=""
NODE_ID=""
NODE_TOKEN=""

INSTALL_BIN="/usr/local/bin/zmpanel"
CTL_BIN="/usr/local/bin/zmpanel-ctl"
CONF_DIR="/etc/zmpanel"
ENV_FILE="${CONF_DIR}/install.env"

# Action flags
ACTION="install"         # install | uninstall | status | logs | restart | info | upgrade
UNINSTALL_MODE="prompt"  # prompt | panel | purge | all
PURGE_DATA=0
REMOVE_APPS=""           # comma-separated: nginx,redis,postgresql,mongodb,uv,docker
PURGE_DOCKER_DATA=0
ASSUME_YES=0

# Colors (safe when non-TTY)
if [ -t 1 ]; then
  C_GREEN='\033[0;32m'
  C_YELLOW='\033[1;33m'
  C_RED='\033[0;31m'
  C_CYAN='\033[0;36m'
  C_RESET='\033[0m'
else
  C_GREEN=''; C_YELLOW=''; C_RED=''; C_CYAN=''; C_RESET=''
fi

info() { printf "${C_CYAN}[zmpanel]${C_RESET} %s\n" "$*"; }
ok()   { printf "${C_GREEN}[  OK  ]${C_RESET} %s\n" "$*"; }
warn() { printf "${C_YELLOW}[ WARN ]${C_RESET} %s\n" "$*"; }
err()  { printf "${C_RED}[ ERR  ]${C_RESET} %s\n" "$*" >&2; }

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    err "请使用 root 权限运行（例如在命令前加 sudo）。"
    exit 1
  fi
}

detect_os() {
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    . /etc/os-release
    case "${ID:-}" in
      alpine) echo "alpine" ;;
      ubuntu) echo "ubuntu" ;;
      debian) echo "debian" ;;
      *)
        case "${ID_LIKE:-}" in
          *debian*) echo "debian" ;;
          *alpine*) echo "alpine" ;;
          *) echo "${ID:-linux}" ;;
        esac
        ;;
    esac
  else
    echo "linux"
  fi
}

detect_target() {
  local arch
  arch="$(uname -m)"
  case "$arch" in
    x86_64|amd64)  echo "x86_64-unknown-linux-musl" ;;
    aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
    *)
      err "暂不支持的 CPU 架构: $arch（支持 x86_64 / aarch64）"
      exit 1
      ;;
  esac
}

proxy_url() {
  local url="$1"
  if [ -n "$GH_PROXY" ]; then
    local p="${GH_PROXY%/}"
    echo "${p}/${url}"
  else
    echo "$url"
  fi
}

http_download() {
  local url="$1"
  local dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fSL --connect-timeout 15 --max-time 300 -o "$dest" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    err "系统缺少 curl 或 wget，无法下载文件。请先安装 curl。"
    exit 1
  fi
}

verify_sha256() {
  local file="$1"
  local expected="$2"
  local actual=""
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  else
    warn "系统未找到 sha256sum，跳过完整性校验。"
    return 0
  fi
  if [ "$actual" != "$expected" ]; then
    err "SHA256 校验失败！期望: $expected，实际: $actual"
    exit 1
  fi
  ok "SHA256 校验通过 ($actual)"
}

load_saved_env() {
  if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE"
  fi
}

save_install_env() {
  mkdir -p "$CONF_DIR"
  cat > "$ENV_FILE" <<EOF
MODE="${MODE}"
PORT="${PORT}"
HOST="${HOST}"
DATA_DIR="${DATA_DIR}"
SERVICE_NAME="${SERVICE_NAME}"
REPO="${REPO}"
EOF
  chmod 0600 "$ENV_FILE"
}

install_ctl_script() {
  # Persist this script as /usr/local/bin/zmpanel-ctl so users can manage/uninstall anytime offline
  local self_path="${BASH_SOURCE[0]:-}"
  if [ -n "$self_path" ] && [ -f "$self_path" ]; then
    install -m 0755 "$self_path" "$CTL_BIN"
  else
    # Invoked via `curl ... | bash`; fetch a local copy into /usr/local/bin/zmpanel-ctl
    local script_url
    script_url="$(proxy_url "https://raw.githubusercontent.com/${REPO}/main/scripts/install.sh")"
    if http_download "$script_url" "${CTL_BIN}.tmp" 2>/dev/null; then
      mv "${CTL_BIN}.tmp" "$CTL_BIN"
      chmod 0755 "$CTL_BIN"
    else
      rm -f "${CTL_BIN}.tmp"
      warn "未能缓存控制脚本到 ${CTL_BIN}（不影响面板运行）"
    fi
  fi
}

download_release_binary() {
  local target="$1"
  local out_bin="$2"
  local asset_name="zmpanel-${target}"
  local base_url

  if [ "$VERSION" = "latest" ]; then
    base_url="https://github.com/${REPO}/releases/latest/download"
  else
    local tag="$VERSION"
    case "$tag" in v*) ;; *) tag="v${tag}" ;; esac
    base_url="https://github.com/${REPO}/releases/download/${tag}"
  fi

  local bin_url sha_url tmp_sha
  bin_url="$(proxy_url "${base_url}/${asset_name}")"
  sha_url="$(proxy_url "${base_url}/${asset_name}.sha256")"
  tmp_sha="${out_bin}.sha256"

  info "正在下载静态单二进制: ${bin_url}"
  if ! http_download "$bin_url" "$out_bin"; then
    err "下载预编译二进制失败。"
    echo "提示："
    echo "  1) 若尚未在 GitHub Releases 发布版本，请先在仓库推送 tag（如 git tag v0.1.0 && git push origin v0.1.0）触发自动构建；"
    echo "  2) 若国内访问 GitHub 慢，可设置代理前缀：GH_PROXY=\"https://ghfast.top/\" sudo -E bash ..."
    echo "  3) 也可使用本地二进制安装：sudo bash install.sh --bin ./zmpanel"
    echo "  4) 或从源码编译安装：sudo bash install.sh --from-source"
    exit 1
  fi

  if http_download "$sha_url" "$tmp_sha" 2>/dev/null; then
    local expected_sha
    expected_sha="$(awk '{print $1}' "$tmp_sha")"
    rm -f "$tmp_sha"
    if [ -n "$expected_sha" ]; then
      verify_sha256 "$out_bin" "$expected_sha"
    fi
  else
    rm -f "$tmp_sha"
    warn "未获取到 .sha256 校验文件，继续安装。"
  fi
  chmod 0755 "$out_bin"
}

build_from_source() {
  local out_bin="$1"
  local tmp_src
  tmp_src="$(mktemp -d /tmp/zmpanel-src.XXXXXX)"
  trap 'rm -rf "$tmp_src"' RETURN

  for cmd in git cargo pnpm; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      err "从源码构建需要安装 $cmd。"
      exit 1
    fi
  done

  info "正在克隆源码仓库 https://github.com/${REPO}.git ..."
  git clone --depth 1 "$(proxy_url "https://github.com/${REPO}.git")" "$tmp_src"
  info "正在构建 Vue 3 前端..."
  (cd "$tmp_src/web" && pnpm install --frozen-lockfile && pnpm build)
  info "正在编译 Rust Release 单二进制..."
  (cd "$tmp_src" && cargo build --release -p zmpanel-server)
  cp "$tmp_src/target/release/zmpanel" "$out_bin"
  chmod 0755 "$out_bin"
}

stop_service_unit() {
  local svc="$1"
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl stop "$svc" 2>/dev/null || true
    systemctl disable "$svc" 2>/dev/null || true
    rm -f "/etc/systemd/system/${svc}.service"
  fi
  if command -v rc-service >/dev/null 2>&1; then
    rc-service "$svc" stop 2>/dev/null || true
    rc-update del "$svc" default 2>/dev/null || true
    rm -f "/etc/init.d/${svc}"
  fi
  if [ -f "/run/${svc}.pid" ]; then
    local pid
    pid="$(cat "/run/${svc}.pid" 2>/dev/null || true)"
    if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
    fi
    rm -f "/run/${svc}.pid"
  fi
}

cleanup_panel_system_resources() {
  # 1. Remove zmpanel's isolated nftables table (never touches host's own tables)
  if command -v nft >/dev/null 2>&1; then
    if nft list table inet zmpanel >/dev/null 2>&1; then
      nft delete table inet zmpanel 2>/dev/null || true
      ok "已清理面板创建的 nftables 规则表 (inet zmpanel)"
    fi
  fi

  # 2. Disable & remove zmpanel's swapfile if created by panel
  if [ -f /swapfile.zmpanel ]; then
    swapoff /swapfile.zmpanel 2>/dev/null || true
    rm -f /swapfile.zmpanel
    ok "已关闭并删除面板创建的 /swapfile.zmpanel"
  fi
}

detect_installed_services() {
  local found=()
  command -v nginx >/dev/null 2>&1 && found+=("nginx")
  (command -v redis-server >/dev/null 2>&1 || command -v redis-cli >/dev/null 2>&1) && found+=("redis")
  (command -v postgres >/dev/null 2>&1 || command -v psql >/dev/null 2>&1) && found+=("postgresql")
  (command -v mongod >/dev/null 2>&1) && found+=("mongodb")
  (command -v uv >/dev/null 2>&1 || [ -x /root/.local/bin/uv ] || [ -x /usr/local/bin/uv ]) && found+=("uv")
  command -v docker >/dev/null 2>&1 && found+=("docker")
  echo "${found[*]:-}"
}

uninstall_one_app() {
  local app="$1"
  local os="$2"
  info "正在卸载关联服务: ${app} (${os}) ..."

  case "$app" in
    nginx)
      systemctl stop nginx 2>/dev/null || rc-service nginx stop 2>/dev/null || true
      if [ "$os" = "alpine" ]; then
        apk del nginx 2>/dev/null || true
      elif command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y --auto-remove 'nginx*' 2>/dev/null || true
      fi
      ok "已卸载 Nginx"
      ;;
    redis)
      systemctl stop redis-server redis 2>/dev/null || rc-service redis stop 2>/dev/null || true
      if [ "$os" = "alpine" ]; then
        apk del redis 2>/dev/null || true
      elif command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y --auto-remove 'redis*' 2>/dev/null || true
      fi
      ok "已卸载 Redis"
      ;;
    postgresql|postgres)
      systemctl stop postgresql 2>/dev/null || rc-service postgresql stop 2>/dev/null || true
      if [ "$os" = "alpine" ]; then
        apk del postgresql postgresql-client 2>/dev/null || true
      elif command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y --auto-remove 'postgresql*' 2>/dev/null || true
      fi
      ok "已卸载 PostgreSQL"
      ;;
    mongodb|mongo)
      systemctl stop mongod mongodb 2>/dev/null || true
      if command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y --auto-remove 'mongodb*' 'mongod*' 2>/dev/null || true
      fi
      ok "已卸载 MongoDB"
      ;;
    uv)
      rm -f /usr/local/bin/uv /usr/local/bin/uvx /root/.local/bin/uv /root/.local/bin/uvx "${HOME}/.local/bin/uv" "${HOME}/.local/bin/uvx"
      rm -rf /root/.cache/uv /root/.local/share/uv "${HOME}/.cache/uv" "${HOME}/.local/share/uv"
      ok "已卸载 Python uv 管理器"
      ;;
    docker)
      # Gracefully stop running containers before uninstalling Docker engine
      if command -v docker >/dev/null 2>&1; then
        local cids
        cids="$(docker ps -q 2>/dev/null || true)"
        if [ -n "$cids" ]; then
          info "正在停止运行中的 Docker 容器..."
          docker stop $cids 2>/dev/null || true
        fi
      fi
      systemctl stop docker docker.socket containerd 2>/dev/null || rc-service docker stop 2>/dev/null || true
      if [ "$os" = "alpine" ]; then
        apk del docker docker-cli docker-cli-compose containerd 2>/dev/null || true
      elif command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y --auto-remove \
          docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin \
          docker.io docker-compose docker-doc podman-docker 2>/dev/null || true
      fi
      if [ "$PURGE_DOCKER_DATA" -eq 1 ]; then
        rm -rf /var/lib/docker /var/lib/containerd /etc/docker
        ok "已卸载 Docker 并清空 /var/lib/docker 镜像与卷数据"
      else
        ok "已卸载 Docker 引擎（默认保留 /var/lib/docker 数据；如需连带删除请加 --purge-docker-data）"
      fi
      ;;
    *)
      warn "跳过未知软件项: $app"
      ;;
  esac
}

run_uninstall() {
  need_root
  load_saved_env
  local os
  os="$(detect_os)"

  local detected_apps
  detected_apps="$(detect_installed_services)"

  # Interactive menu when no explicit mode is passed and TTY is available
  if [ "$UNINSTALL_MODE" = "prompt" ]; then
    if [ "$ASSUME_YES" -eq 1 ] || [ ! -t 0 ]; then
      UNINSTALL_MODE="panel"
    else
      echo "============================================================"
      echo "  zmpanel 卸载向导"
      echo "============================================================"
      echo "  1) 仅卸载面板本身（保留网站数据、Nginx、Docker、数据库与配置目录 ${DATA_DIR}）"
      echo "  2) 卸载面板 + 清理面板数据、nftables 规则表与 Swap 文件（保留业务软件）"
      echo "  3) 完全卸载（卸载面板 + 清理数据 + 自主选择卸载 Nginx / Redis / PG / Mongo / uv / Docker）"
      echo "  0) 取消退出"
      echo "------------------------------------------------------------"
      [ -n "$detected_apps" ] && echo "  当前本机检测到可管理软件: ${detected_apps}"
      printf "请选择 [0-3] (默认 1): "
      local choice=""
      read -r choice </dev/tty || true
      case "${choice:-1}" in
        1) UNINSTALL_MODE="panel" ;;
        2) UNINSTALL_MODE="purge" ;;
        3) UNINSTALL_MODE="all" ;;
        0) info "已取消卸载。"; exit 0 ;;
        *) err "无效选项: $choice"; exit 1 ;;
      esac
    fi
  fi

  if [ "$UNINSTALL_MODE" = "purge" ] || [ "$UNINSTALL_MODE" = "all" ]; then
    PURGE_DATA=1
  fi

  # Determine apps to remove in `all` mode
  if [ "$UNINSTALL_MODE" = "all" ] && [ -z "$REMOVE_APPS" ]; then
    if [ -z "$detected_apps" ]; then
      info "本机未检测到已安装的 Nginx / Redis / PostgreSQL / MongoDB / uv / Docker。"
    elif [ "$ASSUME_YES" -eq 1 ] || [ ! -t 0 ]; then
      REMOVE_APPS="$(echo "$detected_apps" | tr ' ' ',')"
    else
      echo ""
      echo "本机检测到以下软件: ${detected_apps}"
      printf "请输入要一并卸载的软件（逗号分隔，输入 all 卸载全部，直接回车跳过软件卸载）: "
      local app_input=""
      read -r app_input </dev/tty || true
      if [ "$app_input" = "all" ]; then
        REMOVE_APPS="$(echo "$detected_apps" | tr ' ' ',')"
      else
        REMOVE_APPS="$app_input"
      fi

      case ",${REMOVE_APPS}," in
        *,docker,*)
          printf "是否同时删除 /var/lib/docker 中的镜像、容器卷等数据？[y/N]: "
          local dk_ans=""
          read -r dk_ans </dev/tty || true
          case "$dk_ans" in y|Y|yes|YES) PURGE_DOCKER_DATA=1 ;; esac
          ;;
      esac
    fi
  fi

  # Final confirmation for destructive modes
  if [ "$ASSUME_YES" -ne 1 ] && [ -t 0 ]; then
    echo ""
    warn "即将执行卸载操作："
    echo "  - 卸载模式     : ${UNINSTALL_MODE}"
    echo "  - 清理面板数据 : $( [ "$PURGE_DATA" -eq 1 ] && echo "是 (${DATA_DIR})" || echo "否 (保留 ${DATA_DIR})" )"
    [ -n "$REMOVE_APPS" ] && echo "  - 卸载关联服务 : ${REMOVE_APPS}"
    [ "$PURGE_DOCKER_DATA" -eq 1 ] && echo "  - 清空 Docker  : 是 (/var/lib/docker)"
    printf "确认继续吗？[y/N]: "
    local confirm=""
    read -r confirm </dev/tty || true
    case "$confirm" in
      y|Y|yes|YES) ;;
      *) info "已取消卸载。"; exit 0 ;;
    esac
  fi

  info "正在停止并移除 zmpanel 服务..."
  stop_service_unit "zmpanel"
  stop_service_unit "zmpanel-agent"
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl daemon-reload 2>/dev/null || true
  fi

  # Terminate any remaining zmpanel process
  pkill -f "/usr/local/bin/zmpanel" 2>/dev/null || true

  rm -f "$INSTALL_BIN" "${INSTALL_BIN}.bak" "$CTL_BIN"
  rm -rf "$CONF_DIR"
  ok "已移除面板二进制与系统服务"

  if [ "$PURGE_DATA" -eq 1 ]; then
    cleanup_panel_system_resources
    if [ -n "$DATA_DIR" ] && [ "$DATA_DIR" != "/" ] && [ -d "$DATA_DIR" ]; then
      rm -rf "$DATA_DIR"
      ok "已删除面板数据目录: ${DATA_DIR}"
    fi
    rm -f /var/log/zmpanel.log /var/log/zmpanel-agent.log
  else
    info "已保留面板数据目录: ${DATA_DIR}（网站文件、SSL 证书与数据库完好无损）"
  fi

  if [ -n "$REMOVE_APPS" ]; then
    local IFS=','
    for app in $REMOVE_APPS; do
      app="$(echo "$app" | tr -d '[:space:]')"
      [ -n "$app" ] && uninstall_one_app "$app" "$os"
    done
  fi

  echo ""
  ok "zmpanel 卸载完成！"
  exit 0
}

run_ctl_subcommand() {
  load_saved_env
  local svc="${SERVICE_NAME:-zmpanel}"
  case "$ACTION" in
    status)
      if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
        systemctl status "$svc" --no-pager || true
      elif command -v rc-service >/dev/null 2>&1; then
        rc-service "$svc" status || true
      else
        pgrep -af "$INSTALL_BIN" || echo "$svc 未运行"
      fi
      ;;
    restart)
      need_root
      if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
        systemctl restart "$svc"
      elif command -v rc-service >/dev/null 2>&1; then
        rc-service "$svc" restart
      else
        err "无 systemd/OpenRC 容器环境请手动重启进程。"
        exit 1
      fi
      ok "已重启 ${svc}"
      ;;
    logs|info)
      if command -v journalctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
        journalctl -u "$svc" -n 40 --no-pager
      elif [ -f "/var/log/${svc}.log" ]; then
        tail -n 40 "/var/log/${svc}.log"
      elif [ -f "${DATA_DIR}/${svc}.log" ]; then
        tail -n 40 "${DATA_DIR}/${svc}.log"
      else
        warn "未找到日志文件。"
      fi
      ;;
    upgrade)
      need_root
      local target tmp_bin
      target="$(detect_target)"
      tmp_bin="$(mktemp /tmp/zmpanel-upgrade.XXXXXX)"
      download_release_binary "$target" "$tmp_bin"
      "$tmp_bin" --version
      [ -f "$INSTALL_BIN" ] && cp "$INSTALL_BIN" "${INSTALL_BIN}.bak"
      mv "$tmp_bin" "$INSTALL_BIN"
      chmod 0755 "$INSTALL_BIN"
      if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
        systemctl restart "$svc"
      elif command -v rc-service >/dev/null 2>&1; then
        rc-service "$svc" restart
      fi
      ok "升级完成！已重启 ${svc}（旧版本备份于 ${INSTALL_BIN}.bak）"
      ;;
  esac
  exit 0
}

print_help() {
  cat <<'EOF'
zmpanel 一键安装与管理脚本 (Debian / Ubuntu / Alpine Linux)

用法:
  install.sh [安装选项]
  zmpanel-ctl <status|logs|info|restart|upgrade|uninstall> [选项]

安装选项:
  --mode <standalone|master|agent>   运行模式 (默认 standalone)
  --port <端口>                      监听端口 (默认 18080)
  --host <地址>                      监听地址 (默认 0.0.0.0)
  --data-dir <目录>                  数据目录 (默认 /opt/zmpanel/data)
  --entry <入口路径>                 自定义安全入口 (留空自动随机生成)
  --admin-pass <密码>                自定义初始密码 (留空自动生成)
  --version <latest|v0.1.0>          指定下载版本 (默认 latest)
  --bin <本地路径>                   使用本地已有二进制文件安装
  --from-source                      直接从 GitHub 克隆并本地编译安装
  --master-url <wss://...>           Agent 模式主控 WebSocket 地址
  --node-id <node_xxx>               Agent 模式节点 ID
  --node-token <token>               Agent 模式节点令牌

卸载选项:
  --uninstall                        打开交互式卸载菜单（可任选仅卸面板或连带卸服务）
  --uninstall-panel                  仅卸载面板本身，保留数据与所有业务服务
  --purge-panel                      卸载面板 + 删除面板数据 + 清理 nftables/Swap
  --uninstall-all                    完全卸载（面板 + 数据 + 可选卸载关联软件）
  --remove-apps <列表>               配合 --uninstall-all 指定卸载软件 (nginx,redis,postgresql,mongodb,uv,docker)
  --purge-docker-data                卸载 Docker 时一并删除 /var/lib/docker
  --yes, -y                          跳过交互确认直接执行

环境变量:
  GH_PROXY="https://ghfast.top/"     GitHub 下载加速前缀 (国内服务器可选)
EOF
}

# Parse subcommand (for zmpanel-ctl)
if [ $# -gt 0 ]; then
  case "$1" in
    uninstall) ACTION="uninstall"; shift ;;
    status|logs|info|restart|upgrade) ACTION="$1"; shift ;;
    help|-h|--help) print_help; exit 0 ;;
  esac
fi

while [ $# -gt 0 ]; do
  case "$1" in
    --mode) MODE="$2"; shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --host) HOST="$2"; shift 2 ;;
    --data-dir) DATA_DIR="$2"; shift 2 ;;
    --entry) ENTRY="$2"; shift 2 ;;
    --admin-pass) ADMIN_PASS="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --bin) BIN_SRC="$2"; shift 2 ;;
    --from-source) FROM_SOURCE=1; shift ;;
    --master-url) MASTER_URL="$2"; shift 2 ;;
    --node-id) NODE_ID="$2"; shift 2 ;;
    --node-token) NODE_TOKEN="$2"; shift 2 ;;
    --uninstall) ACTION="uninstall"; UNINSTALL_MODE="prompt"; shift ;;
    --uninstall-panel) ACTION="uninstall"; UNINSTALL_MODE="panel"; shift ;;
    --purge-panel) ACTION="uninstall"; UNINSTALL_MODE="purge"; shift ;;
    --purge-data) PURGE_DATA=1; shift ;;
    --uninstall-all) ACTION="uninstall"; UNINSTALL_MODE="all"; shift ;;
    --remove-apps) REMOVE_APPS="$2"; shift 2 ;;
    --purge-docker-data) PURGE_DOCKER_DATA=1; shift ;;
    -y|--yes) ASSUME_YES=1; shift ;;
    -h|--help) print_help; exit 0 ;;
    *) err "未知参数: $1（使用 --help 查看说明）"; exit 1 ;;
  esac
done

if [ "$ACTION" = "uninstall" ]; then
  run_uninstall
fi

if [ "$ACTION" != "install" ]; then
  run_ctl_subcommand
fi

# ======================== Install Flow ========================
need_root

SERVICE_NAME="zmpanel"
[ "$MODE" = "agent" ] && SERVICE_NAME="zmpanel-agent"

if [ "$MODE" = "agent" ]; then
  if [ -z "$MASTER_URL" ] || [ -z "$NODE_TOKEN" ]; then
    err "Agent 模式必须提供 --master-url 和 --node-token"
    exit 1
  fi
fi

OS_NAME="$(detect_os)"
TARGET="$(detect_target)"
info "检测到系统: ${OS_NAME} | 目标架构: ${TARGET} | 安装模式: ${MODE}"

TMP_BIN="$(mktemp /tmp/zmpanel-install.XXXXXX)"
trap 'rm -f "$TMP_BIN"' EXIT

if [ -n "$BIN_SRC" ]; then
  if [ ! -f "$BIN_SRC" ]; then
    err "未找到指定的本地二进制文件: ${BIN_SRC}"
    exit 1
  fi
  cp "$BIN_SRC" "$TMP_BIN"
  chmod 0755 "$TMP_BIN"
elif [ "$FROM_SOURCE" -eq 1 ]; then
  build_from_source "$TMP_BIN"
else
  download_release_binary "$TARGET" "$TMP_BIN"
fi

# Smoke-test binary before replacing
"$TMP_BIN" --version >/dev/null

# Stop running service before replacing executable
stop_service_unit "$SERVICE_NAME"

install -m 0755 "$TMP_BIN" "$INSTALL_BIN"
mkdir -p "$DATA_DIR"
save_install_env
install_ctl_script

EXEC_ARGS="--mode ${MODE} --data-dir ${DATA_DIR}"
if [ "$MODE" = "agent" ]; then
  EXEC_ARGS="${EXEC_ARGS} --master-url ${MASTER_URL} --node-token ${NODE_TOKEN}"
else
  EXEC_ARGS="${EXEC_ARGS} --host ${HOST} --port ${PORT}"
  [ -n "$ENTRY" ] && EXEC_ARGS="${EXEC_ARGS} --entry ${ENTRY}"
  [ -n "$ADMIN_PASS" ] && EXEC_ARGS="${EXEC_ARGS} --admin-pass ${ADMIN_PASS}"
fi

if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
  cat > "/etc/systemd/system/${SERVICE_NAME}.service" <<EOF
[Unit]
Description=zmpanel (${MODE})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=ZMPANEL_NODE_ID=${NODE_ID}
ExecStart=${INSTALL_BIN} ${EXEC_ARGS}
Restart=always
RestartSec=3
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable --now "$SERVICE_NAME"
  sleep 1
  ok "已安装并启动 systemd 服务: ${SERVICE_NAME}"
  echo "------------------------------------------------------------"
  journalctl -u "$SERVICE_NAME" -n 20 --no-pager || true
  echo "------------------------------------------------------------"

elif command -v rc-service >/dev/null 2>&1; then
  cat > "/etc/init.d/${SERVICE_NAME}" <<EOF
#!/sbin/openrc-run
name="${SERVICE_NAME}"
command="${INSTALL_BIN}"
command_args="${EXEC_ARGS}"
command_background=true
pidfile="/run/${SERVICE_NAME}.pid"
output_log="/var/log/${SERVICE_NAME}.log"
error_log="/var/log/${SERVICE_NAME}.log"
export ZMPANEL_NODE_ID="${NODE_ID}"

depend() {
    need net
}
EOF
  chmod +x "/etc/init.d/${SERVICE_NAME}"
  rc-update add "$SERVICE_NAME" default
  rc-service "$SERVICE_NAME" restart
  sleep 1
  ok "已安装并启动 Alpine OpenRC 服务: ${SERVICE_NAME}"
  echo "------------------------------------------------------------"
  tail -n 20 "/var/log/${SERVICE_NAME}.log" 2>/dev/null || true
  echo "------------------------------------------------------------"

else
  warn "未检测到 systemd / OpenRC（常见于精简 Docker/LXC 容器），改用后台守护方式启动..."
  # shellcheck disable=SC2086
  ZMPANEL_NODE_ID="$NODE_ID" nohup "$INSTALL_BIN" $EXEC_ARGS >"${DATA_DIR}/${SERVICE_NAME}.log" 2>&1 &
  echo $! > "/run/${SERVICE_NAME}.pid" 2>/dev/null || true
  sleep 1
  echo "------------------------------------------------------------"
  cat "${DATA_DIR}/${SERVICE_NAME}.log" 2>/dev/null || true
  echo "------------------------------------------------------------"
fi

echo ""
ok "安装完成！常用运维与卸载命令："
echo "  查看状态 / 入口密码 : sudo zmpanel-ctl info"
echo "  重启面板服务        : sudo zmpanel-ctl restart"
echo "  一键升级到最新版    : sudo zmpanel-ctl upgrade"
echo "  卸载向导（单卸/全卸）: sudo zmpanel-ctl uninstall"
