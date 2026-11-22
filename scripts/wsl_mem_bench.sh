#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
PROJ_DIR="$(pwd)"
export CARGO_TARGET_DIR="/tmp/zmpanel-wsl-target"
export NO_PROXY="*"

# Use thin LTO for fast release benchmarking (same runtime footprint as fat LTO, ~3x faster compile)
export CARGO_PROFILE_RELEASE_LTO=thin
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=4

echo "=== [1/3] 编译 Linux Release 裁剪单二进制 ==="
cargo build --release --manifest-path "$PROJ_DIR/Cargo.toml" -p zmpanel-server
BIN="$CARGO_TARGET_DIR/release/zmpanel"
ls -lh "$BIN"
echo "文件信息: $(file "$BIN" 2>/dev/null || echo 'N/A')"

read_mem() {
  local pid="$1"
  local label="$2"
  local rss hwm anon file_rss pss
  rss="$(awk '/^VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
  hwm="$(awk '/^VmHWM:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
  anon="$(awk '/^RssAnon:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
  file_rss="$(awk '/^RssFile:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
  pss="$(awk '/^Pss:/ {print $2}' "/proc/$pid/smaps_rollup" 2>/dev/null || echo 0)"
  awk -v lbl="$label" -v rss="$rss" -v hwm="$hwm" -v anon="$anon" -v fr="$file_rss" -v pss="$pss" \
    'BEGIN {
      printf "  %-28s VmRSS=%6.2f MB | PSS=%6.2f MB | RssAnon(私有堆)=%6.2f MB | RssFile(代码映射)=%6.2f MB | 峰值VmHWM=%6.2f MB\n",
        lbl, rss/1024, pss/1024, anon/1024, fr/1024, hwm/1024
    }'
}

MASTER_DIR="$(mktemp -d /tmp/zmpanel-bench-master.XXXXXX)"
AGENT_DIR="$(mktemp -d /tmp/zmpanel-bench-agent.XXXXXX)"
PORT=18995
ENTRY="zm_bench"
PASS="BenchPass123!"

cleanup() {
  for p in "${AGENT_PID:-}" "${MASTER_PID:-}"; do
    if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then
      kill "$p" 2>/dev/null || true
      wait "$p" 2>/dev/null || true
    fi
  done
  rm -rf "$MASTER_DIR" "$AGENT_DIR"
}
trap cleanup EXIT

echo ""
echo "=== [2/3] 实测 Master / Standalone 模式内存占用 ==="
"$BIN" --mode master --host 127.0.0.1 --port "$PORT" --data-dir "$MASTER_DIR" --entry "$ENTRY" --admin-pass "$PASS" >"$MASTER_DIR/master.log" 2>&1 &
MASTER_PID=$!

for _ in $(seq 1 40); do
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/$ENTRY/api/auth/status"; then
    break
  fi
  sleep 0.2
done
sleep 2
read_mem "$MASTER_PID" "Master 启动待机 (登录前)"

# Perform login + fetch snapshot + docker status + store apps
LOGIN_JSON="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"admin\",\"password\":\"$PASS\"}")"
TOKEN="$(printf '%s' "$LOGIN_JSON" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')"
curl -sf "http://127.0.0.1:$PORT/$ENTRY/api/system/snapshot" -H "Authorization: Bearer $TOKEN" >/dev/null
DOCKER_ST="$(curl -sf "http://127.0.0.1:$PORT/$ENTRY/api/docker/status" -H "Authorization: Bearer $TOKEN")"
curl -sf "http://127.0.0.1:$PORT/$ENTRY/api/store/apps" -H "Authorization: Bearer $TOKEN" >/dev/null
sleep 1
read_mem "$MASTER_PID" "Master 登录后活跃访问"

echo "  宿主机环境探测: $(printf '%s' "$DOCKER_ST" | sed -n 's/.*"virt_label":"\([^"]*\)".*/\1/p') (docker_mode=$(printf '%s' "$DOCKER_ST" | sed -n 's/.*"docker_mode":"\([^"]*\)".*/\1/p'))"

echo ""
echo "=== [3/3] 实测纯受控 NAT Agent 模式内存占用 ==="
NODE_JSON="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/cluster/nodes" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"bench-nat-agent","group_name":"基准测试"}')"
NODE_ID="$(printf '%s' "$NODE_JSON" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')"
NODE_TOKEN="$(printf '%s' "$NODE_JSON" | sed -n 's/.*"node_token":"\([^"]*\)".*/\1/p')"

ZMPANEL_NODE_ID="$NODE_ID" "$BIN" --mode agent \
  --master-url "ws://127.0.0.1:$PORT/$ENTRY/api/agent/ws" \
  --node-token "$NODE_TOKEN" \
  --data-dir "$AGENT_DIR" >"$AGENT_DIR/agent.log" 2>&1 &
AGENT_PID=$!

for _ in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$PORT/$ENTRY/api/cluster/nodes" -H "Authorization: Bearer $TOKEN" | grep -q '"status":"online"'; then
    break
  fi
  sleep 0.2
done
sleep 2
read_mem "$AGENT_PID" "Agent 已连主控待机 (心跳中)"

# Dispatch one command over reverse WS and check memory after work
curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/cluster/batch/exec" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"node_ids\":[\"$NODE_ID\"],\"command\":\"uname -a\"}" >/dev/null
sleep 1
read_mem "$AGENT_PID" "Agent 执行远程指令后"
read_mem "$MASTER_PID" "Master 稳定运行 (带1节点)"

echo ""
echo "=== 内存基准测试完成 ==="
