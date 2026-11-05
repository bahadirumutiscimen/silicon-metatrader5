#!/bin/bash
# Install MetaTrader 5 into the persistent prefix (first start only).
# The MetaQuotes installer downloads the terminal itself, so it always fetches
# the current build. Retries because the installer is flaky under emulation.
set -euo pipefail
SMT5_COMPONENT=installer
# shellcheck source=common.sh
source /siliconmt5/scripts/common.sh

if [ -f "$MT5_EXE" ]; then
  log "MT5 already installed"
  exit 0
fi

setup=/tmp/mt5setup.exe
attempts="${SMT5_INSTALL_ATTEMPTS:-3}"
wait_steps=90   # × 10 s for terminal64.exe to appear

for attempt in $(seq 1 "$attempts"); do
  set_phase installing_mt5 "attempt $attempt/$attempts"
  curl -fsSL --retry 3 -o "$setup" "$MT5_URL"
  wine "$setup" /auto >>"$SMT5_LOG_DIR/installer.log" 2>&1 &
  setup_pid=$!

  # Wait up to 15 min for the terminal binary to appear …
  for _ in $(seq 1 "$wait_steps"); do
    [ -f "$MT5_EXE" ] && break
    sleep 10
  done
  # … then up to 3 min for the installer to finish copying files.
  if [ -f "$MT5_EXE" ]; then
    for _ in $(seq 1 36); do
      pgrep -f 'mt5setup\.exe' >/dev/null 2>&1 || break
      sleep 5
    done
  fi

  # The installer starts the terminal when it is done; the supervisor will
  # start it again with our config, so close everything here.
  wine taskkill /IM mt5setup.exe /F >/dev/null 2>&1 || true
  wine taskkill /IM terminal64.exe >/dev/null 2>&1 || true
  sleep 5
  wine taskkill /IM terminal64.exe /F >/dev/null 2>&1 || true
  kill "$setup_pid" 2>/dev/null || true
  wait "$setup_pid" 2>/dev/null || true
  rm -f "$setup"

  if [ -f "$MT5_EXE" ]; then
    log "MT5 installed"
    exit 0
  fi
  log "MT5 install attempt $attempt failed"
  sleep 5
done

log "ERROR: terminal64.exe not found after $attempts attempts"
find "$WINEPREFIX/drive_c" \( -iname 'terminal64.exe' -o -iname 'metatrader*' \) 2>/dev/null | head -20 || true
exit 1
