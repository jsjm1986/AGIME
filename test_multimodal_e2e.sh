#!/usr/bin/env bash
# E2E 真实任务测试驱动：多模态治理 + tasks + subagent + swarm + thinking
# 用法: bash test_multimodal_e2e.sh
# 前置: target/release/agimed.exe 已编译；keyring(service=agime)中有 mimo 凭证
set -uo pipefail

SECRET="e2e-test-secret-key"
HOST="127.0.0.1"
PORT="3456"
BASE="http://${HOST}:${PORT}"
WORKDIR="/tmp/agime_test"
PNG_B64=$(cat /tmp/agime_test/red_px_b64.txt)
RESULTS="/tmp/agime_test/results.log"
: > "$RESULTS"

log()  { echo -e "$@" | tee -a "$RESULTS"; }
pass() { log "  ✅ PASS: $*"; }
fail() { log "  ❌ FAIL: $*"; }

hdr() { echo -e "\n========== $* ==========" | tee -a "$RESULTS"; }

# --- helper: call /reply, collect SSE for up to N seconds, dump raw to file ---
reply() {
  local sid="$1" payload="$2" out="$3" secs="${4:-90}"
  curl -sN -m "$secs" -X POST "${BASE}/reply" \
    -H "Content-Type: application/json" \
    -H "X-Secret-Key: ${SECRET}" \
    -d "$payload" > "$out" 2>&1
}

start_agent() {
  curl -s -X POST "${BASE}/agent/start" \
    -H "Content-Type: application/json" \
    -H "X-Secret-Key: ${SECRET}" \
    -d "{\"working_dir\":\"${WORKDIR}\"}"
}

set_cfg() {
  local key="$1" val="$2"
  curl -s -X POST "${BASE}/config/upsert" \
    -H "Content-Type: application/json" \
    -H "X-Secret-Key: ${SECRET}" \
    -d "{\"key\":\"${key}\",\"value\":${val},\"is_secret\":false}"
}

read_cfg() {
  local key="$1"
  curl -s -X POST "${BASE}/config/read" \
    -H "Content-Type: application/json" \
    -H "X-Secret-Key: ${SECRET}" \
    -d "{\"key\":\"${key}\",\"is_secret\":false}"
}

update_provider() {
  local sid="$1" provider="$2" model="$3"
  curl -s -X POST "${BASE}/agent/update_provider" \
    -H "Content-Type: application/json" \
    -H "X-Secret-Key: ${SECRET}" \
    -d "{\"session_id\":\"${sid}\",\"provider\":\"${provider}\",\"model\":\"${model}\"}"
}

now() { date +%s; }
msg_user_text() { # $1=text
  printf '{"role":"user","created":%d,"content":[{"type":"text","text":%s}]}' "$(now)" "$(jq -Rn --arg t "$1" '$t')"
}
msg_user_image_text() { # $1=text $2=b64
  printf '{"role":"user","created":%d,"content":[{"type":"text","text":%s},{"type":"image","data":"%s","mime_type":"image/png"}]}' \
    "$(now)" "$(jq -Rn --arg t "$1" '$t')" "$2"
}

# ============ wait for server ============
hdr "等待 server 就绪 ${BASE}"
for i in $(seq 1 30); do
  if curl -s -m 2 "${BASE}/status" >/dev/null 2>&1; then
    log "server up after ${i}s"; break
  fi
  sleep 1
done
curl -s -m 3 "${BASE}/status"; echo

echo "DONE_SETUP"
