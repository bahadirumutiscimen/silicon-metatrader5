#!/bin/bash
# Start the MT5 terminal in portable mode with our startup config.
# The config is re-applied only when the image template or one of the
# SMT5_MAXBARS / SMT5_ALGO_TRADING / SMT5_DLL_IMPORT values changes, so tweaks
# made inside MT5 (e.g. the Algo Trading button) survive plain restarts.
set -euo pipefail
SMT5_COMPONENT=mt5
# shellcheck source=common.sh
source /siliconmt5/scripts/common.sh

if [ ! -f "$MT5_EXE" ]; then
  log "ERROR: terminal64.exe not found" >&2
  exit 1
fi

cfg_src="$SMT5_HOME/config/mt5cfg.ini"
cfg_dst="$MT5_DIR/mt5cfg.ini"
cfg_hash_file="$SMT5_STATE_DIR/mt5cfg.sha256"
cfg_tmp="$(mktemp)"
trap 'rm -f "$cfg_tmp"' EXIT

cp "$cfg_src" "$cfg_tmp"
if [ -n "${SMT5_MAXBARS:-}" ]; then
  case "$SMT5_MAXBARS" in
    ''|*[!0-9]*) log "ignoring invalid SMT5_MAXBARS=$SMT5_MAXBARS" ;;
    *) sed -i "s/^MaxBars=.*/MaxBars=${SMT5_MAXBARS}/" "$cfg_tmp" ;;
  esac
fi

set_key() { # set_key KEY VALUE  (first match in the file)
  sed -i "0,/^$1=.*/s//$1=$2/" "$cfg_tmp"
}
bool() { case "${1:-}" in 1|true|yes|on) echo 1 ;; 0|false|no|off) echo 0 ;; *) echo "" ;; esac; }
algo="$(bool "${SMT5_ALGO_TRADING:-}")"
if [ -n "$algo" ]; then set_key Enabled "$algo"; set_key AllowLiveTrading "$algo"; fi
dll="$(bool "${SMT5_DLL_IMPORT:-}")"
[ -n "$dll" ] && set_key AllowDllImport "$dll"

new_hash="$(sha256sum "$cfg_tmp" | awk '{print $1}')"
old_hash="$(cat "$cfg_hash_file" 2>/dev/null || true)"
if [ ! -f "$cfg_dst" ] || [ "$new_hash" != "$old_hash" ]; then
  cp "$cfg_tmp" "$cfg_dst"
  echo "$new_hash" >"$cfg_hash_file"
  log "mt5cfg.ini synchronized ($(grep -E '^(MaxBars|Enabled|AllowLiveTrading|AllowDllImport)=' "$cfg_tmp" | tr '\n' ' '))"
fi

# No sound card in the container: without this Wine's ALSA/Pulse drivers print
# dozens of "ALSA lib ... cannot find card '0'" lines per minute. SMT5_AUDIO=1
# keeps them.
if [ "${SMT5_AUDIO:-0}" != "1" ]; then
  export WINEDLLOVERRIDES="${WINEDLLOVERRIDES:+$WINEDLLOVERRIDES;}winealsa.drv,winepulse.drv,wineoss.drv=d"
fi

# A fixed DPI (96 = Windows' 100 %); otherwise Wine may derive it from the
# virtual screen and MT5 comes out oversized in noVNC. SMT5_DPI: 72-288.
dpi="${SMT5_DPI:-96}"
case "$dpi" in ''|*[!0-9]*) dpi=96 ;; esac
{ [ "$dpi" -ge 72 ] && [ "$dpi" -le 288 ]; } || dpi=96
wine reg add 'HKCU\Control Panel\Desktop' /v LogPixels /t REG_DWORD /d "$dpi" /f >/dev/null 2>&1 || true
wine reg add 'HKLM\System\CurrentControlSet\Hardware Profiles\Current\Software\Fonts' /v LogPixels /t REG_DWORD /d "$dpi" /f >/dev/null 2>&1 || true

# no GPU in the container: software GL, without Mesa's EGL/DRI warnings
export LIBGL_ALWAYS_SOFTWARE=1 EGL_LOG_LEVEL=fatal MESA_LOG_LEVEL=error

cd "$MT5_DIR"
exec wine terminal64.exe /portable /config:mt5cfg.ini
