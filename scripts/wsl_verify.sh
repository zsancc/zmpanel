#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
PROJ_DIR="$(pwd)"
export CARGO_TARGET_DIR="/tmp/zmpanel-wsl-target"
export NO_PROXY="*"

echo "=== [1/3] Linux 原生 cargo test (阶段 1~4 全量 6 项端到端集成测试) ==="
cargo test --manifest-path "$PROJ_DIR/Cargo.toml"

echo ""
echo "=== [2/3] 编译 Linux 二进制 ==="
cargo build --manifest-path "$PROJ_DIR/Cargo.toml" -p zmpanel-server
BIN="$CARGO_TARGET_DIR/debug/zmpanel"

echo ""
echo "=== [3/3] 真机启动 Master + 独立 NAT Agent 双进程反向 WS 联调实测 ==="
MASTER_DIR="$(mktemp -d /tmp/zmpanel-master.XXXXXX)"
AGENT_DIR="$(mktemp -d /tmp/zmpanel-agent.XXXXXX)"
PORT=18994
ENTRY="zm_cluster_test"
PASS="WslCluster123!"

cleanup() {
  local rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "--- 失败诊断 (exit=$rc) ---"
    echo "[Master log]"; cat "$MASTER_DIR/master.log" 2>/dev/null || true
    echo "[Agent log]"; cat "$AGENT_DIR/agent.log" 2>/dev/null || true
  fi
  for p in "${AGENT_PID:-}" "${MASTER_PID:-}"; do
    if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then
      kill "$p" 2>/dev/null || true
      wait "$p" 2>/dev/null || true
    fi
  done
  rm -rf "$MASTER_DIR" "$AGENT_DIR"
  exit "$rc"
}
trap cleanup EXIT

"$BIN" --mode master --host 127.0.0.1 --port "$PORT" --data-dir "$MASTER_DIR" --entry "$ENTRY" --admin-pass "$PASS" >"$MASTER_DIR/master.log" 2>&1 &
MASTER_PID=$!

for _ in $(seq 1 40); do
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/$ENTRY/api/auth/status"; then
    break
  fi
  sleep 0.2
done

LOGIN_JSON="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"admin\",\"password\":\"$PASS\"}")"
TOKEN="$(printf '%s' "$LOGIN_JSON" | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')"
[ -n "$TOKEN" ]

# 1. Create a NAT Agent node on Master
NODE_JSON="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/cluster/nodes" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"wsl-nat-agent","group_name":"NAT测试组"}')"
NODE_ID="$(printf '%s' "$NODE_JSON" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')"
NODE_TOKEN="$(printf '%s' "$NODE_JSON" | sed -n 's/.*"node_token":"\([^"]*\)".*/\1/p')"
echo "1) 主控注册受控节点成功: id=$NODE_ID"

# 2. Launch real standalone Agent process connecting outbound to Master
ZMPANEL_NODE_ID="$NODE_ID" "$BIN" --mode agent \
  --master-url "ws://127.0.0.1:$PORT/$ENTRY/api/agent/ws" \
  --node-token "$NODE_TOKEN" \
  --data-dir "$AGENT_DIR" >"$AGENT_DIR/agent.log" 2>&1 &
AGENT_PID=$!

# Wait for agent online
ONLINE=0
for _ in $(seq 1 30); do
  if curl -sf "http://127.0.0.1:$PORT/$ENTRY/api/cluster/nodes" -H "Authorization: Bearer $TOKEN" | grep -q '"status":"online"'; then
    ONLINE=1
    break
  fi
  sleep 0.2
done
[ "$ONLINE" = "1" ]

AGENT_RSS_KB="$(awk '/VmRSS:/ {print $2}' "/proc/$AGENT_PID/status" 2>/dev/null || echo 0)"
AGENT_RSS_MB="$(awk "BEGIN {printf \"%.2f\", $AGENT_RSS_KB / 1024}")"
echo "2) 独立 NAT Agent 主动反连主控上线成功！Agent (Debug) 实测 VmRSS = ${AGENT_RSS_MB} MB (${AGENT_RSS_KB} KB)"

# 3. Master dispatches batch command to Agent over reverse WS
BATCH_OUT="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/cluster/batch/exec" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"node_ids\":[\"$NODE_ID\"],\"command\":\"uname -s && echo wsl_cluster_ok\"}")"
echo "$BATCH_OUT" | grep -q "wsl_cluster_ok"
echo "3) 主控穿透反向通道批量下发命令实测通过 (命中 wsl_cluster_ok)"

# 4. Master pulls backup from Agent into central repository
PULL_OUT="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/cluster/nodes/$NODE_ID/pull-backup" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"wsl-agent-bk","include_websites":true,"include_docker":true,"include_ssl":true,"include_panel_db":true}')"
BK_ID="$(printf '%s' "$PULL_OUT" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')"
[ -n "$BK_ID" ]
echo "4) 主控从 NAT 小鸡一键拉取备份至中央仓库成功 (bk_id=$BK_ID)"

# 5. Master pushes backup back to target Agent for cross-machine restore
RESTORE_OUT="$(curl -sf -X POST "http://127.0.0.1:$PORT/$ENTRY/api/cluster/backups/$BK_ID/push-restore" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d "{\"target_node_id\":\"$NODE_ID\"}")"
echo "$RESTORE_OUT" | grep -q '"ok":true'
echo "5) 主控一键推送跨机恢复到目标节点成功 -> $(printf '%s' "$RESTORE_OUT" | cut -c1-140)..."

echo ""
echo "=== ALL STAGE 1~4 WSL LINUX MASTER+AGENT CLUSTER CHECKS PASSED ==="
