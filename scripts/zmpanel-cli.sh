#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# zmpanel 面板命令行管理工具 (安装位置: /usr/local/bin/zmpanel)
# 直接运行 `zmpanel` 打开交互式管理菜单，或使用快捷子命令：
#   zmpanel info | start | stop | restart | status | logs
#   zmpanel port [新端口]
#   zmpanel host [新地址]
#   zmpanel entry [新入口|random|none]
#   zmpanel user [新用户名]
#   zmpanel passwd [新密码]
#   zmpanel disable-2fa
#   zmpanel upgrade
#   zmpanel uninstall [--uninstall-panel|--purge-panel|--uninstall-all] [--remove-apps ...] [--yes]
# ==============================================================================

REPO="${ZMPANEL_REPO:-zsancc/zmpanel}"
GH_PROXY="${GH_PROXY:-}"
VERSION="${ZMPANEL_VERSION:-latest}"

DAEMON_BIN="/opt/zmpanel/bin/zmpaneld"
CLI_BIN="/usr/local/bin/zmpanel"
CONF_DIR="/etc/zmpanel"
ENV_FILE="${CONF_DIR}/install.env"

MODE="standalone"
PORT="18080"
HOST="0.0.0.0"
DATA_DIR="/opt/zmpanel/data"
SERVICE_NAME="zmpanel"
MASTER_URL=""
NODE_ID=""
NODE_TOKEN=""

if [ -t 1 ]; then
  C_GREEN='\033[0;32m'
  C_YELLOW='\033[1;33m'
  C_RED='\033[0;31m'
  C_CYAN='\033[0;36m'
  C_BOLD='\033[1m'
  C_RESET='\033[0m'
else
  C_GREEN=''; C_YELLOW=''; C_RED=''; C_CYAN=''; C_BOLD=''; C_RESET=''
fi

info() { printf "${C_CYAN}[zmpanel]${C_RESET} %s\n" "$*"; }
ok()   { printf "${C_GREEN}[  OK  ]${C_RESET} %s\n" "$*"; }
warn() { printf "${C_YELLOW}[ WARN ]${C_RESET} %s\n" "$*"; }
err()  { printf "${C_RED}[ ERR  ]${C_RESET} %s\n" "$*" >&2; }

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    err "该操作需要 root 权限，请使用 sudo zmpanel ..."
    exit 1
  fi
}

load_env() {
  if [ -f "$ENV_FILE" ]; then
    # shellcheck disable=SC1090
    . "$ENV_FILE"
  fi
  if [ ! -x "$DAEMON_BIN" ] && [ -x /usr/local/bin/zmpaneld ]; then
    DAEMON_BIN="/usr/local/bin/zmpaneld"
  fi
}

save_env() {
  mkdir -p "$CONF_DIR"
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
    *) err "不支持的 CPU 架构: $arch"; exit 1 ;;
  esac
}

detect_local_ipv4() {
  local ip=""
  if command -v ip >/dev/null 2>&1; then
    ip="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") {print $(i+1); exit}}' || true)"
  fi
  if [ -z "$ip" ] && command -v hostname >/dev/null 2>&1; then
    ip="$(hostname -I 2>/dev/null | awk '{print $1}' || true)"
  fi
  echo "${ip:-127.0.0.1}"
}

build_exec_args() {
  local args="--mode ${MODE} --data-dir ${DATA_DIR}"
  if [ "$MODE" = "agent" ]; then
    args="${args} --master-url ${MASTER_URL} --node-token ${NODE_TOKEN}"
  else
    args="${args} --host ${HOST} --port ${PORT}"
  fi
  echo "$args"
}

rewrite_service_unit() {
  local exec_args
  exec_args="$(build_exec_args)"

  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    cat > "/etc/systemd/system/${SERVICE_NAME}.service" <<EOF
[Unit]
Description=zmpanel (${MODE})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=ZMPANEL_NODE_ID=${NODE_ID}
ExecStart=${DAEMON_BIN} ${exec_args}
Restart=always
RestartSec=3
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
  elif command -v rc-service >/dev/null 2>&1; then
    cat > "/etc/init.d/${SERVICE_NAME}" <<EOF
#!/sbin/openrc-run
name="${SERVICE_NAME}"
command="${DAEMON_BIN}"
command_args="${exec_args}"
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
  fi
}

svc_start() {
  need_root
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl start "$SERVICE_NAME"
  elif command -v rc-service >/dev/null 2>&1; then
    rc-service "$SERVICE_NAME" start
  else
    local exec_args
    exec_args="$(build_exec_args)"
    # shellcheck disable=SC2086
    ZMPANEL_NODE_ID="$NODE_ID" nohup "$DAEMON_BIN" $exec_args >"${DATA_DIR}/${SERVICE_NAME}.log" 2>&1 &
    echo $! > "/run/${SERVICE_NAME}.pid" 2>/dev/null || true
  fi
  ok "已启动 ${SERVICE_NAME}"
}

svc_stop() {
  need_root
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl stop "$SERVICE_NAME" 2>/dev/null || true
  elif command -v rc-service >/dev/null 2>&1; then
    rc-service "$SERVICE_NAME" stop 2>/dev/null || true
  fi
  if [ -f "/run/${SERVICE_NAME}.pid" ]; then
    local pid
    pid="$(cat "/run/${SERVICE_NAME}.pid" 2>/dev/null || true)"
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    rm -f "/run/${SERVICE_NAME}.pid"
  fi
  ok "已停止 ${SERVICE_NAME}"
}

svc_restart() {
  need_root
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl restart "$SERVICE_NAME"
  elif command -v rc-service >/dev/null 2>&1; then
    rc-service "$SERVICE_NAME" restart
  else
    svc_stop >/dev/null 2>&1 || true
    svc_start >/dev/null 2>&1
  fi
  ok "已重启 ${SERVICE_NAME}"
}

svc_state_text() {
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
      echo "运行中 (systemd)"
    else
      echo "已停止 (systemd)"
    fi
  elif command -v rc-service >/dev/null 2>&1; then
    if rc-service "$SERVICE_NAME" status >/dev/null 2>&1; then
      echo "运行中 (OpenRC)"
    else
      echo "已停止 (OpenRC)"
    fi
  else
    if pgrep -f "$DAEMON_BIN" >/dev/null 2>&1; then
      echo "运行中 (容器后台)"
    else
      echo "已停止"
    fi
  fi
}

cmd_info() {
  local ver="" entry="" username="" totp="false"
  if [ -x "$DAEMON_BIN" ] && [ -f "${DATA_DIR}/zmpanel.db" ]; then
    local out
    out="$("$DAEMON_BIN" --data-dir "$DATA_DIR" info 2>/dev/null || true)"
    ver="$(printf '%s\n' "$out" | awk -F= '$1=="VERSION"{print $2}')"
    entry="$(printf '%s\n' "$out" | awk -F= '$1=="ENTRY"{print $2}')"
    username="$(printf '%s\n' "$out" | awk -F= '$1=="USERNAME"{print $2}')"
    totp="$(printf '%s\n' "$out" | awk -F= '$1=="TOTP_ENABLED"{print $2}')"
  fi
  [ -z "$ver" ] && [ -x "$DAEMON_BIN" ] && ver="$("$DAEMON_BIN" --version 2>/dev/null | awk '{print $2}' || echo 'unknown')"

  local ip
  ip="$(detect_local_ipv4)"
  local disp_host="$ip"
  [ "$HOST" != "0.0.0.0" ] && [ "$HOST" != "::" ] && disp_host="$HOST"

  echo "============================================================"
  printf "  ${C_BOLD}zmpanel 面板信息${C_RESET} (v%s)\n" "${ver:-unknown}"
  echo "============================================================"
  echo "  运行模式   : ${MODE}"
  echo "  服务状态   : $(svc_state_text)"
  echo "  监听地址   : ${HOST}:${PORT}"
  if [ "$MODE" != "agent" ]; then
    echo "  内网/本机  : http://${disp_host}:${PORT}${entry}"
    [ "$HOST" = "0.0.0.0" ] && echo "  公网访问   : http://<你的服务器公网IP>:${PORT}${entry}"
    echo "  安全入口   : ${entry:-(已关闭，直接访问 /)}"
    echo "  管理员账号 : ${username:-(未知)}"
    echo "  2FA 验证   : $( [ "$totp" = "true" ] && echo "已开启" || echo "未开启" )"
  else
    echo "  主控地址   : ${MASTER_URL}"
    echo "  节点 ID    : ${NODE_ID}"
  fi
  echo "  数据目录   : ${DATA_DIR}"
  echo "  服务端程序 : ${DAEMON_BIN}"
  echo "============================================================"
}

cmd_status() {
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl status "$SERVICE_NAME" --no-pager || true
  elif command -v rc-service >/dev/null 2>&1; then
    rc-service "$SERVICE_NAME" status || true
  else
    pgrep -af "$DAEMON_BIN" || echo "${SERVICE_NAME} 未运行"
  fi
}

cmd_logs() {
  if command -v journalctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    journalctl -u "$SERVICE_NAME" -n 50 --no-pager
  elif [ -f "/var/log/${SERVICE_NAME}.log" ]; then
    tail -n 50 "/var/log/${SERVICE_NAME}.log"
  elif [ -f "${DATA_DIR}/${SERVICE_NAME}.log" ]; then
    tail -n 50 "${DATA_DIR}/${SERVICE_NAME}.log"
  else
    warn "未找到日志文件。"
  fi
}

cmd_set_port() {
  need_root
  local new_port="${1:-}"
  if [ -z "$new_port" ]; then
    printf "当前监听端口为 %s，请输入新的端口号 (1-65535): " "$PORT"
    read -r new_port </dev/tty || true
  fi
  if ! echo "$new_port" | grep -Eq '^[0-9]+$' || [ "$new_port" -lt 1 ] || [ "$new_port" -gt 65535 ]; then
    err "无效端口号: ${new_port}"
    return 1
  fi
  PORT="$new_port"
  save_env
  rewrite_service_unit
  svc_restart
  ok "面板监听端口已修改为 ${PORT} 并已重启生效！"
  cmd_info
}

cmd_set_host() {
  need_root
  local new_host="${1:-}"
  if [ -z "$new_host" ]; then
    echo "常用地址：0.0.0.0（允许外网访问） 或 127.0.0.1（仅本机/反代访问）"
    printf "当前监听地址为 %s，请输入新的监听地址: " "$HOST"
    read -r new_host </dev/tty || true
  fi
  if [ -z "$new_host" ]; then
    err "监听地址不能为空。"
    return 1
  fi
  HOST="$new_host"
  save_env
  rewrite_service_unit
  svc_restart
  ok "面板监听地址已修改为 ${HOST} 并已重启生效！"
  cmd_info
}

cmd_set_entry() {
  need_root
  local new_entry="${1:-}"
  if [ -z "$new_entry" ]; then
    echo "说明：输入自定义路径（如 mypanel），输入 random 生成随机安全入口，输入 none 关闭入口限制"
    printf "请输入新的安全入口: "
    read -r new_entry </dev/tty || true
  fi
  if [ -z "$new_entry" ]; then
    err "未输入安全入口，已取消。"
    return 1
  fi
  "$DAEMON_BIN" --data-dir "$DATA_DIR" set-entry "$new_entry"
  svc_restart
  cmd_info
}

cmd_set_username() {
  need_root
  local new_user="${1:-}"
  if [ -z "$new_user" ]; then
    printf "请输入新的管理员用户名 (3-32位字母/数字/下划线): "
    read -r new_user </dev/tty || true
  fi
  if [ -z "$new_user" ]; then
    err "用户名不能为空。"
    return 1
  fi
  "$DAEMON_BIN" --data-dir "$DATA_DIR" set-username "$new_user"
}

cmd_reset_password() {
  need_root
  local new_pass="${1:-}"
  if [ -z "$new_pass" ]; then
    printf "请输入新密码（至少6位，直接回车则自动生成 14 位强随机密码）: "
    read -r new_pass </dev/tty || true
  fi
  if [ -n "$new_pass" ]; then
    "$DAEMON_BIN" --data-dir "$DATA_DIR" reset-password --password "$new_pass"
  else
    "$DAEMON_BIN" --data-dir "$DATA_DIR" reset-password
  fi
}

cmd_disable_2fa() {
  need_root
  "$DAEMON_BIN" --data-dir "$DATA_DIR" disable-2fa
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
    err "缺少 curl 或 wget"
    exit 1
  fi
}

cmd_upgrade() {
  need_root
  local target asset base_url bin_url sha_url cli_url tmp_bin tmp_sha
  target="$(detect_target)"
  asset="zmpanel-${target}"
  if [ "$VERSION" = "latest" ]; then
    base_url="https://github.com/${REPO}/releases/latest/download"
  else
    local tag="$VERSION"
    case "$tag" in v*) ;; *) tag="v${tag}" ;; esac
    base_url="https://github.com/${REPO}/releases/download/${tag}"
  fi
  bin_url="$(proxy_url "${base_url}/${asset}")"
  sha_url="$(proxy_url "${base_url}/${asset}.sha256")"
  cli_url="$(proxy_url "https://raw.githubusercontent.com/${REPO}/main/scripts/zmpanel-cli.sh")"

  tmp_bin="$(mktemp /tmp/zmpanel-upgrade.XXXXXX)"
  tmp_sha="${tmp_bin}.sha256"
  trap 'rm -f "$tmp_bin" "$tmp_sha"' RETURN

  info "正在下载最新版服务端: ${bin_url}"
  http_download "$bin_url" "$tmp_bin"
  if http_download "$sha_url" "$tmp_sha" 2>/dev/null; then
    local expected actual
    expected="$(awk '{print $1}' "$tmp_sha")"
    if command -v sha256sum >/dev/null 2>&1 && [ -n "$expected" ]; then
      actual="$(sha256sum "$tmp_bin" | awk '{print $1}')"
      if [ "$actual" != "$expected" ]; then
        err "SHA256 校验失败！期望 ${expected}，实际 ${actual}"
        exit 1
      fi
      ok "SHA256 校验通过"
    fi
  fi

  chmod 0755 "$tmp_bin"
  "$tmp_bin" --version
  mkdir -p "$(dirname "$DAEMON_BIN")"
  [ -f "$DAEMON_BIN" ] && cp "$DAEMON_BIN" "${DAEMON_BIN}.bak"
  mv "$tmp_bin" "$DAEMON_BIN"
  chmod 0755 "$DAEMON_BIN"

  # Also refresh zmpanel CLI script
  if http_download "$cli_url" "${CLI_BIN}.tmp" 2>/dev/null; then
    chmod 0755 "${CLI_BIN}.tmp"
    mv "${CLI_BIN}.tmp" "$CLI_BIN"
  else
    rm -f "${CLI_BIN}.tmp"
  fi

  rewrite_service_unit
  svc_restart
  ok "升级完成！（旧版已备份至 ${DAEMON_BIN}.bak）"
  cmd_info
}

# ======================== Uninstall Logic ========================
stop_and_remove_unit() {
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
    [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    rm -f "/run/${svc}.pid"
  fi
}

cleanup_panel_system_resources() {
  if command -v nft >/dev/null 2>&1; then
    if nft list table inet zmpanel >/dev/null 2>&1; then
      nft delete table inet zmpanel 2>/dev/null || true
      ok "已清理面板创建的 nftables 规则表 (inet zmpanel)"
    fi
  fi
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
  local app="$1" os="$2" purge_docker="$3"
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
      if command -v docker >/dev/null 2>&1; then
        local cids
        cids="$(docker ps -q 2>/dev/null || true)"
        [ -n "$cids" ] && docker stop $cids 2>/dev/null || true
      fi
      systemctl stop docker docker.socket containerd 2>/dev/null || rc-service docker stop 2>/dev/null || true
      if [ "$os" = "alpine" ]; then
        apk del docker docker-cli docker-cli-compose containerd 2>/dev/null || true
      elif command -v apt-get >/dev/null 2>&1; then
        DEBIAN_FRONTEND=noninteractive apt-get purge -y --auto-remove \
          docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin \
          docker.io docker-compose docker-doc podman-docker 2>/dev/null || true
      fi
      if [ "$purge_docker" -eq 1 ]; then
        rm -rf /var/lib/docker /var/lib/containerd /etc/docker
        ok "已卸载 Docker 并清空 /var/lib/docker"
      else
        ok "已卸载 Docker 引擎（默认保留 /var/lib/docker 数据；如需连带删除请加 --purge-docker-data）"
      fi
      ;;
    *)
      warn "跳过未知软件项: $app"
      ;;
  esac
}

cmd_uninstall() {
  need_root
  local umode="prompt" purge_data=0 remove_apps="" purge_docker=0 yes=0

  while [ $# -gt 0 ]; do
    case "$1" in
      --uninstall|--uninstall-panel) umode="panel"; shift ;;
      --purge-panel) umode="purge"; shift ;;
      --purge-data) purge_data=1; shift ;;
      --uninstall-all) umode="all"; shift ;;
      --remove-apps) remove_apps="$2"; shift 2 ;;
      --purge-docker-data) purge_docker=1; shift ;;
      -y|--yes) yes=1; shift ;;
      *) err "未知卸载参数: $1"; exit 1 ;;
    esac
  done

  local os detected_apps
  os="$(detect_os)"
  detected_apps="$(detect_installed_services)"

  if [ "$umode" = "prompt" ]; then
    if [ "$yes" -eq 1 ] || [ ! -t 0 ]; then
      umode="panel"
    else
      echo "============================================================"
      echo "  zmpanel 卸载向导"
      echo "============================================================"
      echo "  1) 仅卸载面板本身（保留网站数据、Nginx、Docker、数据库与 ${DATA_DIR}）"
      echo "  2) 卸载面板 + 清理面板数据、nftables 规则表与 Swap 文件（保留业务软件）"
      echo "  3) 完全卸载（卸载面板 + 清理数据 + 自主选择卸载 Nginx / Redis / PG / Mongo / uv / Docker）"
      echo "  0) 取消退出"
      echo "------------------------------------------------------------"
      [ -n "$detected_apps" ] && echo "  本机检测到关联服务: ${detected_apps}"
      printf "请选择 [0-3] (默认 1): "
      local choice=""
      read -r choice </dev/tty || true
      case "${choice:-1}" in
        1) umode="panel" ;;
        2) umode="purge" ;;
        3) umode="all" ;;
        0) info "已取消卸载。"; return 0 ;;
        *) err "无效选项: $choice"; return 1 ;;
      esac
    fi
  fi

  [ "$umode" = "purge" ] || [ "$umode" = "all" ] && purge_data=1

  if [ "$umode" = "all" ] && [ -z "$remove_apps" ]; then
    if [ -z "$detected_apps" ]; then
      info "本机未检测到已安装的 Nginx / Redis / PostgreSQL / MongoDB / uv / Docker。"
    elif [ "$yes" -eq 1 ] || [ ! -t 0 ]; then
      remove_apps="$(echo "$detected_apps" | tr ' ' ',')"
    else
      echo ""
      echo "本机检测到以下软件: ${detected_apps}"
      printf "请输入要一并卸载的软件（逗号分隔，输入 all 卸载全部，直接回车跳过软件卸载）: "
      local app_in=""
      read -r app_in </dev/tty || true
      if [ "$app_in" = "all" ]; then
        remove_apps="$(echo "$detected_apps" | tr ' ' ',')"
      else
        remove_apps="$app_in"
      fi
      case ",${remove_apps}," in
        *,docker,*)
          printf "是否同时删除 /var/lib/docker 镜像与数据卷？[y/N]: "
          local dk=""
          read -r dk </dev/tty || true
          case "$dk" in y|Y|yes|YES) purge_docker=1 ;; esac
          ;;
      esac
    fi
  fi

  if [ "$yes" -ne 1 ] && [ -t 0 ]; then
    echo ""
    warn "即将执行卸载："
    echo "  - 卸载模式     : ${umode}"
    echo "  - 清理面板数据 : $( [ "$purge_data" -eq 1 ] && echo "是 (${DATA_DIR})" || echo "否 (保留 ${DATA_DIR})" )"
    [ -n "$remove_apps" ] && echo "  - 卸载关联服务 : ${remove_apps}"
    [ "$purge_docker" -eq 1 ] && echo "  - 清空 Docker  : 是 (/var/lib/docker)"
    printf "确认继续吗？[y/N]: "
    local cf=""
    read -r cf </dev/tty || true
    case "$cf" in y|Y|yes|YES) ;; *) info "已取消卸载。"; return 0 ;; esac
  fi

  info "正在停止并移除 zmpanel 服务..."
  stop_and_remove_unit "zmpanel"
  stop_and_remove_unit "zmpanel-agent"
  if command -v systemctl >/dev/null 2>&1 && [ -d /run/systemd/system ]; then
    systemctl daemon-reload 2>/dev/null || true
  fi
  pkill -f "/opt/zmpanel/bin/zmpaneld" 2>/dev/null || true
  pkill -f "/usr/local/bin/zmpaneld" 2>/dev/null || true

  rm -rf /opt/zmpanel/bin "$CONF_DIR"
  rm -f /usr/local/bin/zmpanel-ctl /usr/local/bin/zmpanel.bak "$CLI_BIN"
  ok "已移除面板程序与系统服务"

  if [ "$purge_data" -eq 1 ]; then
    cleanup_panel_system_resources
    if [ -n "$DATA_DIR" ] && [ "$DATA_DIR" != "/" ] && [ -d "$DATA_DIR" ]; then
      rm -rf "$DATA_DIR"
      ok "已删除面板数据目录: ${DATA_DIR}"
    fi
    rmdir /opt/zmpanel 2>/dev/null || true
    rm -f /var/log/zmpanel.log /var/log/zmpanel-agent.log
  else
    info "已保留面板数据目录: ${DATA_DIR}"
  fi

  if [ -n "$remove_apps" ]; then
    local IFS=','
    for app in $remove_apps; do
      app="$(echo "$app" | tr -d '[:space:]')"
      [ -n "$app" ] && uninstall_one_app "$app" "$os" "$purge_docker"
    done
  fi

  echo ""
  ok "zmpanel 卸载完成！"
  exit 0
}

print_help() {
  cat <<'EOF'
zmpanel 面板命令行管理工具（直接运行 `zmpanel` 打开交互式菜单）

快捷命令:
  zmpanel                      打开交互式编号菜单
  zmpanel info | default | 1   查看面板访问地址、端口、安全入口与管理员账号
  zmpanel start | 2            启动面板服务
  zmpanel stop | 3             停止面板服务
  zmpanel restart | 4          重启面板服务
  zmpanel status               查看 systemd / OpenRC 服务状态
  zmpanel port [端口] | 5      修改面板监听端口并重启生效
  zmpanel host [地址] | 6      修改面板监听地址 (如 0.0.0.0 / 127.0.0.1)
  zmpanel entry [路径] | 7     修改安全入口 (自定义 / random / none)
  zmpanel user [用户名] | 8    修改管理员用户名
  zmpanel passwd [密码] | 9    重置管理员密码 (留空随机生成 14 位强密码)
  zmpanel disable-2fa | 10     应急关闭管理员 2FA 二步验证
  zmpanel logs | 11            查看面板运行日志
  zmpanel upgrade | 12         一键升级面板到最新版本
  zmpanel uninstall | 13       卸载向导（支持单卸面板或连同 Nginx/Docker 等卸载）
EOF
}

run_interactive_menu() {
  local ver="unknown"
  [ -x "$DAEMON_BIN" ] && ver="$("$DAEMON_BIN" --version 2>/dev/null | awk '{print $2}' || echo 'unknown')"

  while true; do
    echo ""
    echo "============================================================"
    printf "  ${C_BOLD}zmpanel 面板管理工具${C_RESET} (v%s · %s)\n" "$ver" "$(svc_state_text)"
    echo "============================================================"
    echo "   (1) 查看面板访问信息（入口地址 / 端口 / 账号 / 状态）"
    echo "   (2) 启动面板服务"
    echo "   (3) 停止面板服务"
    echo "   (4) 重启面板服务"
    echo "   (5) 修改面板监听端口（当前: ${PORT}）"
    echo "   (6) 修改面板监听地址（当前: ${HOST}）"
    echo "   (7) 修改面板安全入口（自定义 / 随机 / 关闭）"
    echo "   (8) 修改管理员用户名"
    echo "   (9) 重置管理员密码"
    echo "  (10) 关闭管理员 2FA（二步验证应急解锁）"
    echo "  (11) 查看面板运行日志"
    echo "  (12) 一键升级到最新版"
    echo "  (13) 卸载面板（可选仅卸面板 / 清理数据 / 连同 Nginx、Docker 等卸载）"
    echo "   (0) 退出"
    echo "============================================================"
    printf "请输入编号 [0-13]: "
    local sel=""
    read -r sel </dev/tty || exit 0
    echo ""
    case "$sel" in
      1)  cmd_info ;;
      2)  svc_start ;;
      3)  svc_stop ;;
      4)  svc_restart ;;
      5)  cmd_set_port ;;
      6)  cmd_set_host ;;
      7)  cmd_set_entry ;;
      8)  cmd_set_username ;;
      9)  cmd_reset_password ;;
      10) cmd_disable_2fa ;;
      11) cmd_logs ;;
      12) cmd_upgrade ;;
      13) cmd_uninstall ;;
      0|q|Q|exit) exit 0 ;;
      *)  warn "无效编号: ${sel}" ;;
    esac
  done
}

# ======================== Main Dispatch ========================
load_env

if [ $# -eq 0 ]; then
  if [ -t 0 ]; then
    run_interactive_menu
  else
    cmd_info
  fi
  exit 0
fi

SUB="$1"
shift || true

case "$SUB" in
  1|info|default)     cmd_info ;;
  2|start)            svc_start ;;
  3|stop)             svc_stop ;;
  4|restart)          svc_restart ;;
  status)             cmd_status ;;
  5|port)             cmd_set_port "${1:-}" ;;
  6|host)             cmd_set_host "${1:-}" ;;
  7|entry)            cmd_set_entry "${1:-}" ;;
  8|user|username)    cmd_set_username "${1:-}" ;;
  9|passwd|password)  cmd_reset_password "${1:-}" ;;
  10|disable-2fa)     cmd_disable_2fa ;;
  11|logs|log)        cmd_logs ;;
  12|upgrade|update)  cmd_upgrade ;;
  13|uninstall)       cmd_uninstall "$@" ;;
  -v|--version)       "$DAEMON_BIN" --version ;;
  -h|--help|help)     print_help ;;
  *)
    err "未知命令: ${SUB}（直接运行 zmpanel 打开菜单，或 zmpanel --help 查看帮助）"
    exit 1
    ;;
esac
