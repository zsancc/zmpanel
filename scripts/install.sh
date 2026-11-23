#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# zmpanel 一键安装 / 卸载引导脚本 (Debian / Ubuntu / Alpine Linux)
# 仓库: https://github.com/zsancc/zmpanel
#
# 一键安装:
#   单机模式 : curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash
#   主控模式 : curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --mode master --port 18080
#   NAT Agent: curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --mode agent --master-url "wss://..." --node-id "node_xxx" --node-token "xxx"
#   国内加速 : GH_PROXY="https://ghfast.top/" curl -fsSL ... | sudo -E bash
#
# 安装完成后直接运行 `zmpanel` 即可打开交互式管理菜单（改端口/地址/入口/账号密码/升级/卸载）。
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

DAEMON_BIN="/opt/zmpanel/bin/zmpaneld"
CLI_BIN="/usr/local/bin/zmpanel"
CONF_DIR="/etc/zmpanel"
ENV_FILE="${CONF_DIR}/install.env"

UNINSTALL_ARGS=()
DO_UNINSTALL=0

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
    *) err "暂不支持的 CPU 架构: $arch（支持 x86_64 / aarch64）"; exit 1 ;;
  esac
}

proxy_url() {
  local url="$1"
  if [ -n "$GH_PROXY" ]; then
    echo "${GH_PROXY%/}/${url}"
  else
    echo "$url"
  fi
}

http_download() {
  local url="$1" dest="$2"
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
  local file="$1" expected="$2" actual=""
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

install_cli_tool() {
  local script_dir=""
  if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  fi

  # Remove deprecated zmpanel-ctl if upgrading from v0.1.0
  rm -f /usr/local/bin/zmpanel-ctl

  if [ -n "$script_dir" ] && [ -f "${script_dir}/zmpanel-cli.sh" ]; then
    install -m 0755 "${script_dir}/zmpanel-cli.sh" "$CLI_BIN"
    ok "已安装管理命令: ${CLI_BIN}"
    return 0
  fi

  local cli_url
  cli_url="$(proxy_url "https://raw.githubusercontent.com/${REPO}/main/scripts/zmpanel-cli.sh")"
  if http_download "$cli_url" "${CLI_BIN}.tmp"; then
    chmod 0755 "${CLI_BIN}.tmp"
    mv "${CLI_BIN}.tmp" "$CLI_BIN"
    ok "已安装管理命令: ${CLI_BIN}"
  else
    rm -f "${CLI_BIN}.tmp"
    err "下载 zmpanel 管理脚本失败 (${cli_url})"
    exit 1
  fi
}

download_release_binary() {
  local target="$1" out_bin="$2"
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
    echo "  1) 若国内访问 GitHub 慢，可设置代理前缀：GH_PROXY=\"https://ghfast.top/\" sudo -E bash ..."
    echo "  2) 也可使用本地二进制安装：sudo bash install.sh --bin ./zmpanel"
    echo "  3) 或从源码编译安装：sudo bash install.sh --from-source"
    exit 1
  fi

  if http_download "$sha_url" "$tmp_sha" 2>/dev/null; then
    local expected_sha
    expected_sha="$(awk '{print $1}' "$tmp_sha")"
    rm -f "$tmp_sha"
    [ -n "$expected_sha" ] && verify_sha256 "$out_bin" "$expected_sha"
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
  fi
  if command -v rc-service >/dev/null 2>&1; then
    rc-service "$svc" stop 2>/dev/null || true
  fi
  if [ -f "/run/${svc}.pid" ]; then
    local pid
    pid="$(cat "/run/${svc}.pid" 2>/dev/null || true)"
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    rm -f "/run/${svc}.pid"
  fi
}

print_help() {
  cat <<'EOF'
zmpanel 一键安装脚本 (Debian / Ubuntu / Alpine Linux)

安装选项:
  --mode <standalone|master|agent>   运行模式 (默认 standalone)
  --port <端口>                      监听端口 (默认 18080)
  --host <地址>                      监听地址 (默认 0.0.0.0)
  --data-dir <目录>                  数据目录 (默认 /opt/zmpanel/data)
  --entry <入口路径>                 自定义安全入口 (留空自动随机生成)
  --admin-pass <密码>                自定义初始密码 (留空自动生成)
  --version <latest|v0.1.1>          指定下载版本 (默认 latest)
  --bin <本地路径>                   使用本地已有二进制文件安装
  --from-source                      直接从 GitHub 克隆并本地编译安装
  --master-url <wss://...>           Agent 模式主控 WebSocket 地址
  --node-id <node_xxx>               Agent 模式节点 ID
  --node-token <token>               Agent 模式节点令牌

卸载选项（安装后也可直接运行 `sudo zmpanel uninstall`）:
  --uninstall | --uninstall-panel | --purge-panel | --uninstall-all
  --remove-apps <nginx,redis,postgresql,mongodb,uv,docker>
  --purge-docker-data | --yes

环境变量:
  GH_PROXY="https://ghfast.top/"     GitHub 下载加速前缀 (国内服务器可选)
EOF
}

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
    --uninstall|--uninstall-panel|--purge-panel|--purge-data|--uninstall-all|--purge-docker-data|-y|--yes)
      DO_UNINSTALL=1; UNINSTALL_ARGS+=("$1"); shift ;;
    --remove-apps)
      DO_UNINSTALL=1; UNINSTALL_ARGS+=("$1" "$2"); shift 2 ;;
    -h|--help|help) print_help; exit 0 ;;
    *) err "未知参数: $1（使用 --help 查看说明）"; exit 1 ;;
  esac
done

# Delegate uninstall to zmpanel CLI tool (fetch temporarily if not installed yet)
if [ "$DO_UNINSTALL" -eq 1 ]; then
  need_root
  if [ -x "$CLI_BIN" ] && head -n 1 "$CLI_BIN" 2>/dev/null | grep -q "bash"; then
    exec "$CLI_BIN" uninstall "${UNINSTALL_ARGS[@]}"
  else
    TMP_CLI="$(mktemp /tmp/zmpanel-cli.XXXXXX)"
    trap 'rm -f "$TMP_CLI"' EXIT
    SCRIPT_DIR=""
    [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ] && SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    if [ -n "$SCRIPT_DIR" ] && [ -f "${SCRIPT_DIR}/zmpanel-cli.sh" ]; then
      cp "${SCRIPT_DIR}/zmpanel-cli.sh" "$TMP_CLI"
    else
      http_download "$(proxy_url "https://raw.githubusercontent.com/${REPO}/main/scripts/zmpanel-cli.sh")" "$TMP_CLI"
    fi
    chmod 0755 "$TMP_CLI"
    exec bash "$TMP_CLI" uninstall "${UNINSTALL_ARGS[@]}"
  fi
fi

# ======================== Install Flow ========================
need_root

# Preserve existing port/host/mode/data_dir if re-installing without explicit flags
if [ -f "$ENV_FILE" ]; then
  # shellcheck disable=SC1090
  . "$ENV_FILE"
fi

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

# Stop running service (and migrate cleanly if old v0.1.0 ran from /usr/local/bin/zmpanel)
stop_service_unit "$SERVICE_NAME"
pkill -f "/usr/local/bin/zmpanel --mode" 2>/dev/null || true

mkdir -p "$(dirname "$DAEMON_BIN")" "$DATA_DIR" "$CONF_DIR"
install -m 0755 "$TMP_BIN" "$DAEMON_BIN"

cat > "$ENV_FILE" <<EOF
MODE="${MODE}"
PORT="${PORT}"
HOST="${HOST}"
DATA_DIR="${DATA_DIR}"
SERVICE_NAME="${SERVICE_NAME}"
REPO="${REPO}"
MASTER_URL="${MASTER_URL}"
NODE_ID="${NODE_ID}"
NODE_TOKEN="${NODE_TOKEN}"
EOF
chmod 0600 "$ENV_FILE"

# Install `zmpanel` interactive CLI management tool at /usr/local/bin/zmpanel
install_cli_tool

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
ExecStart=${DAEMON_BIN} ${EXEC_ARGS}
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
  journalctl -u "$SERVICE_NAME" -n 15 --no-pager || true
  echo "------------------------------------------------------------"

elif command -v rc-service >/dev/null 2>&1; then
  cat > "/etc/init.d/${SERVICE_NAME}" <<EOF
#!/sbin/openrc-run
name="${SERVICE_NAME}"
command="${DAEMON_BIN}"
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
  tail -n 15 "/var/log/${SERVICE_NAME}.log" 2>/dev/null || true
  echo "------------------------------------------------------------"

else
  warn "未检测到 systemd / OpenRC（常见于精简 Docker/LXC 容器），改用后台守护方式启动..."
  # shellcheck disable=SC2086
  ZMPANEL_NODE_ID="$NODE_ID" nohup "$DAEMON_BIN" $EXEC_ARGS >"${DATA_DIR}/${SERVICE_NAME}.log" 2>&1 &
  echo $! > "/run/${SERVICE_NAME}.pid" 2>/dev/null || true
  sleep 1
  echo "------------------------------------------------------------"
  cat "${DATA_DIR}/${SERVICE_NAME}.log" 2>/dev/null || true
  echo "------------------------------------------------------------"
fi

echo ""
"$CLI_BIN" info || true
echo ""
ok "安装完成！直接输入命令即可管理面板（类似宝塔 bt）："
echo "  打开交互式管理菜单  : zmpanel"
echo "  查看入口与账号信息  : zmpanel info"
echo "  修改端口 / 安全入口 : zmpanel port / zmpanel entry"
echo "  修改账号 / 重置密码 : zmpanel user / zmpanel passwd"
echo "  升级 / 卸载面板     : zmpanel upgrade / zmpanel uninstall"
