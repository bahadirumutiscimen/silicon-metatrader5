#!/bin/bash
# SiliconMetaTrader5 runtime supervisor.
#
#   prepare prefix → install MT5 → start terminal → start bridge → watch both
#
# Publishes its phase to $SMT5_STATE_DIR/state.json and honours request files
# written by the status API:
#   maintenance.request    stop MT5 gracefully, write maintenance.ready, wait
#                          until the request disappears, then restart
#   restart-mt5.request    graceful MT5 restart
#   restart-bridge.request bridge restart
set -uo pipefail
SMT5_COMPONENT=supervisor
# shellcheck source=common.sh
source /siliconmt5/scripts/common.sh

mkdir -p "$SMT5_STATE_DIR" "$SMT5_LOG_DIR"
exec > >(tee -a "$SMT5_LOG_DIR/supervisor.log") 2>&1

# Exactly one supervisor: a second one would start a second prefix copy that
# deletes the first one's work.
exec 9>"$SMT5_STATE_DIR/supervisor.lock"
if ! flock -n 9; then
  log "another supervisor is already running (pid $(pgrep -o -f 'scripts/supervisor\.sh' || echo ?)); exiting"
  exit 0
fi
log "supervisor started (pid $$)"
rm -f "$ACK_MAINTENANCE"

BRIDGE_STARTED_AT=0

fit_window() {
  # Map/resize the MT5 window to the virtual screen (noVNC has no WM by default).
  command -v xdotool >/dev/null 2>&1 || return 0
  local wid w h
  for _ in $(seq 1 30); do
    wid="$(xdotool search --class terminal64 2>/dev/null | head -1 || true)"
    [ -n "$wid" ] && break
    sleep 2
  done
  [ -n "${wid:-}" ] || return 0
  w="${SMT5_RESOLUTION%x*}"; h="${SMT5_RESOLUTION#*x}"
  xdotool windowmap "$wid" windowsize "$wid" "$w" "$h" windowmove "$wid" 0 0 2>/dev/null || true
}

start_mt5() {
  set_phase starting_terminal
  # 9>&- : long-lived children must not keep the supervisor lock open
  /siliconmt5/scripts/launch-mt5.sh >>"$SMT5_LOG_DIR/mt5.log" 2>&1 9>&- &
  (sleep 15; fit_window) 9>&- &
  return 0
}

stop_mt5() {
  # Ask nicely first (WM_CLOSE lets MT5 flush history files), then force.
  mt5_running || return 0
  log "stopping MT5"
  wine taskkill /IM terminal64.exe >/dev/null 2>&1 || true
  for _ in $(seq 1 30); do
    mt5_running || return 0
    sleep 1
  done
  log "MT5 did not close in 30s, forcing"
  wine taskkill /IM terminal64.exe /F >/dev/null 2>&1 || true
  sleep 2
  pkill -f 'terminal64\.exe' 2>/dev/null || true
}

start_bridge() {
  set_phase starting_bridge
  /siliconmt5/scripts/launch-bridge.sh >>"$SMT5_LOG_DIR/bridge.log" 2>&1 9>&- &
  BRIDGE_STARTED_AT="$(date +%s)"
}

stop_bridge() {
  pkill -f 'siliconmetatrader5\.server' 2>/dev/null || true
  sleep 2
}

# Restart MT5 *and* the bridge. The MetaTrader5 module inside the bridge's
# Windows Python keeps an IPC channel to the terminal process it attached to;
# after the terminal restarts, initialize() fails with
# (-10002, 'IPC recv failed') until that Python process is restarted too.
# Clients lose their connection either way; keepalive/watchdog clients reconnect.
restart_terminal_and_bridge() {
  stop_bridge
  start_mt5
  sleep 15
  start_bridge
  sleep 10
}

shutdown() {
  set_phase stopping
  stop_bridge
  stop_mt5
  wineserver -k 2>/dev/null || true
  exit 0
}
trap shutdown TERM INT

# --- bring-up -----------------------------------------------------------
translation="$(detect_cpu_translation)"
log "cpu translation: $translation"
if [ "$translation" != "none" ] && [ "${SMT5_ALLOW_USER_EMULATION:-0}" != "1" ]; then
  translation_help
  set_phase error "unsupported CPU translation: $translation (use a full x86_64 VM)"
  sleep infinity & wait
fi

set_phase preparing_prefix
if ! /siliconmt5/scripts/prepare-prefix.sh; then
  fail_phase "wine prefix preparation failed"
  sleep infinity & wait
fi
if ! /siliconmt5/scripts/install-mt5.sh; then
  fail_phase "MetaTrader 5 installation failed"
  sleep infinity & wait
fi

start_mt5
sleep 15
start_bridge

# --- watch loop ---------------------------------------------------------
bridge_misses=0
loops=0
while true; do
  loops=$((loops + 1))
  if [ $((loops % 12)) -eq 1 ]; then rotate_logs; fi  # about once a minute
  if [ -f "$REQ_MAINTENANCE" ]; then
    set_phase maintenance "$(head -c 200 "$REQ_MAINTENANCE" 2>/dev/null)"
    stop_mt5
    touch "$ACK_MAINTENANCE"
    while [ -f "$REQ_MAINTENANCE" ]; do sleep 1; done
    rm -f "$ACK_MAINTENANCE"
    log "maintenance finished"
    restart_terminal_and_bridge
    continue
  fi

  if [ -f "$REQ_RESTART_MT5" ]; then
    rm -f "$REQ_RESTART_MT5"
    log "MT5 restart requested"
    stop_mt5
    restart_terminal_and_bridge
    continue
  fi

  if [ -f "$REQ_RESTART_BRIDGE" ]; then
    rm -f "$REQ_RESTART_BRIDGE"
    log "bridge restart requested"
    stop_bridge
    start_bridge
    sleep 10
    continue
  fi

  if ! mt5_running; then
    log "MT5 not running, restarting it together with the bridge"
    restart_terminal_and_bridge
    continue
  fi

  if port_open "$SMT5_BRIDGE_PORT"; then
    bridge_misses=0
    set_phase running
  else
    bridge_misses=$((bridge_misses + 1))
    # ~30s without a listener and at least 60s since the last start.
    if [ "$bridge_misses" -ge 6 ] && [ $(( $(date +%s) - BRIDGE_STARTED_AT )) -ge 60 ]; then
      log "bridge not listening on :$SMT5_BRIDGE_PORT, restarting"
      stop_bridge
      start_bridge
      bridge_misses=0
    fi
  fi
  sleep 5
done
