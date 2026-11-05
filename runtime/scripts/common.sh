#!/bin/bash
# Shared paths and helpers for the SiliconMetaTrader5 runtime scripts. Source it.
# shellcheck disable=SC2034  # variables are consumed by the scripts sourcing this file

# CPU translation checks (also used at build time, kept in their own file so
# edits here do not invalidate the image's slow Wine/Python build layer).
# shellcheck source=cpu-translation.sh
source /siliconmt5/scripts/cpu-translation.sh

export WINEPREFIX="${WINEPREFIX:-/config/.wine}"
export WINEDEBUG="${WINEDEBUG:--all}"

SMT5_HOME=/siliconmt5
SMT5_STATE_DIR="${SMT5_STATE_DIR:-/config/.siliconmt5}"
SMT5_LOG_DIR="${SMT5_LOG_DIR:-/siliconmt5/logs}"
SMT5_BRIDGE_PORT="${SMT5_BRIDGE_PORT_INTERNAL:-8001}"
SMT5_RESOLUTION="${SMT5_RESOLUTION:-1366x768}"
WIN_PYTHON="${WIN_PYTHON:-C:/Program Files/Python313/python.exe}"
MT5_URL="${MT5_URL:-https://download.mql5.com/cdn/web/metaquotes.ltd/mt5/mt5setup.exe}"

MT5_DIR="$WINEPREFIX/drive_c/Program Files/MetaTrader 5"
MT5_EXE="$MT5_DIR/terminal64.exe"

# Request/ack files shared with the status API (see status/smt5_status.py).
REQ_MAINTENANCE="$SMT5_STATE_DIR/maintenance.request"
ACK_MAINTENANCE="$SMT5_STATE_DIR/maintenance.ready"
REQ_RESTART_MT5="$SMT5_STATE_DIR/restart-mt5.request"
REQ_RESTART_BRIDGE="$SMT5_STATE_DIR/restart-bridge.request"

log() {
  printf '%s [%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "${SMT5_COMPONENT:-runtime}" "$*"
}

_SMT5_LAST_PHASE=""
# set_phase <phase> [detail] — atomically writes $SMT5_STATE_DIR/state.json
set_phase() {
  local phase="$1" detail="${2:-}"
  [ "$phase|$detail" = "$_SMT5_LAST_PHASE" ] && return 0
  _SMT5_LAST_PHASE="$phase|$detail"
  mkdir -p "$SMT5_STATE_DIR"
  python3 - "$phase" "$detail" "$SMT5_STATE_DIR/state.json" <<'PY'
import json, os, sys, time
phase, detail, path = sys.argv[1:4]
tmp = path + ".tmp"
with open(tmp, "w") as fh:
    json.dump({"phase": phase, "detail": detail, "updated": time.time()}, fh)
os.replace(tmp, path)
PY
  log "phase=$phase${detail:+ ($detail)}"
}

# fail_phase <detail> — set phase=error unless a child already wrote a more
# specific error (e.g. "not enough disk space …").
fail_phase() {
  local cur
  cur="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("phase",""))' \
    "$SMT5_STATE_DIR/state.json" 2>/dev/null || true)"
  [ "$cur" = "error" ] || set_phase error "$1"
}

free_mb() {  # free_mb <path> — free space in MB on the filesystem holding <path>
  df -Pk "$1" 2>/dev/null | awk 'NR==2 {print int($4/1024)}'
}

port_open() {  # port_open <port>
  python3 - "$1" <<'PY'
import socket, sys
s = socket.socket()
s.settimeout(1)
sys.exit(0 if s.connect_ex(("127.0.0.1", int(sys.argv[1]))) == 0 else 1)
PY
}

mt5_running() { pgrep -f 'terminal64\.exe' >/dev/null 2>&1; }
bridge_running() { pgrep -f 'siliconmetatrader5\.server' >/dev/null 2>&1; }

# Keep every log under SMT5_LOG_MAX_MB (default 10): the current file is copied
# to <name>.log.1 (its newest part, capped at the limit) and truncated in place.
# Writers use O_APPEND (>> / tee -a), so they continue at the start.
rotate_logs() {
  local max_mb="${SMT5_LOG_MAX_MB:-10}" max f size
  case "$max_mb" in ''|*[!0-9]*) max_mb=10 ;; esac
  [ "$max_mb" -ge 1 ] || max_mb=1
  max=$((max_mb * 1024 * 1024))
  for f in "$SMT5_LOG_DIR"/*.log; do
    [ -f "$f" ] || continue
    size="$(stat -c %s "$f" 2>/dev/null || echo 0)"
    if [ "$size" -gt "$max" ]; then
      tail -c "$max" "$f" >"$f.1.tmp" 2>/dev/null && mv -f "$f.1.tmp" "$f.1"
      : >"$f"
      log "rotated $(basename "$f") ($((size / 1048576)) MB > ${max_mb} MB)"
    fi
  done
}
