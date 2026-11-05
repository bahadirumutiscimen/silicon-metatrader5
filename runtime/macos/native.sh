#!/bin/bash
# SiliconMetaTrader5 native mode ("Mac yerel" / "Linux yerel"): MetaTrader 5 +
# the Python bridge directly in Wine 11, no VM, no Docker.
#   macOS        x86_64 Wine (Gcenx build) under the Mac's own Rosetta 2, a
#                launchd agent; the MT5 window is an ordinary macOS window.
#   Linux x86_64 Wine (Kron4ek WoW64 build), a systemd --user unit (or a
#                detached process). With a desktop (DISPLAY / WAYLAND_DISPLAY)
#                MT5 is a normal window; headless, the supervisor runs its own
#                Xvfb + x11vnc + noVNC on 127.0.0.1:SMT5_DISPLAY_PORT.
#
#   native.sh install     Wine 11 + Wine prefix + Windows Python + MT5 + status API venv
#   native.sh start       install if needed, then start the launchd agent (MT5, bridge, status API)
#   native.sh stop        stop the agent (MT5 closes gracefully first)
#   native.sh restart
#   native.sh import      copy MT5 (broker login, history) from the Güvenli mode container again
#   native.sh status [--json]
#   native.sh run         the supervisor in the foreground (debugging; Ctrl+C stops)
#   native.sh reset       stop + delete the Wine prefix (MT5, broker login, history)
#   native.sh reset-all   stop + delete everything except this script (Wine download, venv too)
#   native.sh uninstall   reset-all + remove the folder itself
#
# Everything lives in ~/Library/Application Support/SiliconMT5 on macOS,
# ${XDG_DATA_HOME:-~/.local/share}/siliconmt5 on Linux (SMT5_NATIVE_HOME):
#   wine/  prefix/  venv/  state/  logs/  runtime/ (copy the launchd agent runs:
#   outside ~/Documents, so macOS never asks for folder access in the background)
#
# Ports: bridge SMT5_BRIDGE_PORT (8001) and status API SMT5_STATUS_PORT (8002),
# always bound to 127.0.0.1. The MT5 window is an ordinary macOS window.
#
# Bash 3.2 (macOS /bin/bash): no associative arrays, no GNU sed/stat/flock.
set -uo pipefail

OS="$(uname -s)"
LABEL=com.siliconmt5.runtime              # macOS launchd label
UNIT=siliconmt5-runtime                   # Linux systemd --user unit
if [ "$OS" = Darwin ]; then
  BASE="${SMT5_NATIVE_HOME:-$HOME/Library/Application Support/SiliconMT5}"
else
  BASE="${SMT5_NATIVE_HOME:-${XDG_DATA_HOME:-$HOME/.local/share}/siliconmt5}"
fi
WINE_HOME="$BASE/wine"
PREFIX="$BASE/prefix"
PROBE_PREFIX="$BASE/native-probe"         # left by the earlier speed probe: reused on install
VENV="$BASE/venv"
STATE="$BASE/state"
LOGS="$BASE/logs"
RT="$BASE/runtime"
PLIST="$STATE/$LABEL.plist"               # not in ~/Library/LaunchAgents: no start at login
ENV_COPY="$STATE/native.env"

# Pinned Wine 11 (its ucrtbase has crealf, which numpy needs).
WINE_PIN="11.21"
if [ "$OS" = Darwin ]; then
  WINE_DESC="Gcenx macOS build, ~215 MB"
  WINE_URL="https://github.com/Gcenx/macOS_Wine_builds/releases/download/11.21/wine-devel-11.21-osx64.tar.xz"
  WINE_SHA256="9909b486657e10ab66c7279371ae1f990345b596b3b1b3a0cf21f28af24fa0ce"
else
  # WoW64 build: 64-bit host libraries only, no i386 multiarch needed
  WINE_DESC="Kron4ek Linux WoW64 build, ~100 MB"
  WINE_URL="https://github.com/Kron4ek/Wine-Builds/releases/download/11.21/wine-11.21-amd64-wow64.tar.xz"
  WINE_SHA256="33f3c5b56b1deb8f6725539ea006f27b5d781b3b4ae6a8a1f492d8af4fb9dc83"
fi
PY_VER="${PYTHON_VERSION:-3.13.9}"
MT5_URL="${MT5_URL:-https://download.mql5.com/cdn/web/metaquotes.ltd/mt5/mt5setup.exe}"
# Docker context of the Güvenli/Docker mode (MT5 is copied from its container)
if [ "$OS" = Darwin ]; then SAFE_CONTEXT="${SMT5_SAFE_CONTEXT:-colima-x86}"; else SAFE_CONTEXT="${SMT5_SAFE_CONTEXT:-}"; fi

MT5_DIR="$PREFIX/drive_c/Program Files/MetaTrader 5"
MT5_EXE="$MT5_DIR/terminal64.exe"
PYDIR="$PREFIX/drive_c/Program Files/Python313"
WINPY='C:\Program Files\Python313\python.exe'

# runtime/ this script belongs to (the repo, or the copy in $RT)
SRC="$(cd "$(dirname "$0")/.." && pwd)"

log() { printf '%s [%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "${COMPONENT:-native}" "$*"; }
die() { log "HATA: $*" >&2; exit 1; }

# --------------------------------------------------------------- helpers ---
need_macos() {  # every command: macOS, or Linux x86_64
  case "$OS/$(uname -m)" in
    Darwin/*|Linux/x86_64) ;;
    *) die "yerel mod yalnız macOS ve Linux x86_64'te çalışır ($OS $(uname -m))" ;;
  esac
}
is_mac() { [ "$OS" = Darwin ]; }

find_wine() {  # find_wine <dir> — the wine loader inside a Wine.app bundle
  [ -d "$1" ] || return 0
  find "$1" -path '*/bin/*' \( -name wine -o -name wine64 \) \( -type f -o -type l \) -perm -u+x 2>/dev/null | sort | head -1
}
wine_major() { "$1" --version 2>/dev/null | sed -n 's/^wine-\([0-9]*\).*/\1/p'; }

resolve_wine() {  # sets WINE / WINESERVER (SMT5_WINE overrides; must be Wine ≥ 11)
  WINE="${SMT5_WINE:-}"
  [ -n "$WINE" ] || WINE="$(find_wine "$WINE_HOME")"
  WINESERVER=""
  if [ -z "$WINE" ] || [ ! -x "$WINE" ]; then WINE=""; return 1; fi
  WINESERVER="$(dirname "$WINE")/wineserver"
  [ -x "$WINESERVER" ] || WINESERVER="$(command -v wineserver || echo wineserver)"
  return 0
}

host_python() {  # a python3 ≥ 3.9 with venv (and, on macOS, without the Xcode tools pop-up)
  local p
  for p in "${SMT5_HOST_PYTHON:-}" /opt/homebrew/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do
    if [ -z "$p" ] || [ ! -x "$p" ]; then continue; fi
    if is_mac && [ "$p" = /usr/bin/python3 ] && ! xcode-select -p >/dev/null 2>&1; then continue; fi
    # Debian/Ubuntu ship venv without ensurepip unless python3-venv is installed
    if "$p" -c 'import sys, venv, ensurepip; sys.exit(0 if sys.version_info >= (3, 9) else 1)' 2>/dev/null; then
      echo "$p"
      return 0
    fi
  done
  return 1
}

# Status API python (venv) for JSON helpers; falls back to the host python.
py() {
  if [ -x "$VENV/bin/python" ]; then "$VENV/bin/python" "$@"; else "$(host_python || echo python3)" "$@"; fi
}

wine_env() {
  export WINEPREFIX="$PREFIX"
  export WINEDEBUG=-all
  # no Mono/Gecko install dialogs, no crash debugger window, no .desktop/menu
  # entries, quiet MoltenVK
  export WINEDLLOVERRIDES="mscoree,mshtml=;winemenubuilder.exe=d;winedbg.exe=d"
  export MVK_CONFIG_LOG_LEVEL=0
}

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | awk '{print $1}'; }
file_bytes() { if is_mac; then stat -f %z "$1"; else stat -c %s "$1"; fi 2>/dev/null || echo 0; }

# .env → exported variables (SMT5_* only; never `source`: values are data)
load_env_file() {
  local f="$1" line key val
  [ -f "$f" ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    key="${line%%=*}"
    val="${line#*=}"
    case "$key" in SMT5_[A-Z0-9_]*) ;; *) continue ;; esac
    case "$key" in *[!A-Z0-9_]*) continue ;; esac
    val="${val%\"}"; val="${val#\"}"; val="${val%\'}"; val="${val#\'}"
    export "$key=$val"
  done <"$f"
}

valid_port() { case "${1:-}" in ''|*[!0-9]*) return 1 ;; esac; [ "$1" -ge 1024 ] && [ "$1" -le 65535 ]; }
ports() {
  BRIDGE_PORT="${SMT5_BRIDGE_PORT:-8001}"; valid_port "$BRIDGE_PORT" || BRIDGE_PORT=8001
  STATUS_PORT="${SMT5_STATUS_PORT:-8002}"; valid_port "$STATUS_PORT" || STATUS_PORT=8002
}
port_open() { (exec 3<>"/dev/tcp/127.0.0.1/$1") >/dev/null 2>&1; }

# The background job: launchd agent (macOS), systemd --user unit, or (Linux
# without a user systemd) a detached process recorded in state/agent.pid.
has_user_systemd() { [ "$OS" = Linux ] && command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; }
agent_loaded() {
  if is_mac; then launchctl list "$LABEL" >/dev/null 2>&1
  elif has_user_systemd && systemctl --user is-active --quiet "$UNIT" 2>/dev/null; then return 0
  else pid_alive "$(cat "$STATE/agent.pid" 2>/dev/null)"; fi
}
agent_pid() {
  if is_mac; then launchctl list "$LABEL" 2>/dev/null | sed -n 's/.*"PID" = \([0-9]*\);.*/\1/p'
  elif has_user_systemd && systemctl --user is-active --quiet "$UNIT" 2>/dev/null; then systemctl --user show -p MainPID --value "$UNIT" 2>/dev/null
  else cat "$STATE/agent.pid" 2>/dev/null; fi
}

installed() { [ -f "$MT5_EXE" ] && [ -f "$PYDIR/python.exe" ] && [ -x "$VENV/bin/python" ] && resolve_wine; }

# ---------------------------------------------------------------- install ---
ensure_wine() {
  if resolve_wine; then
    local major; major="$(wine_major "$WINE")"
    if [ "${major:-0}" -ge 11 ] 2>/dev/null; then
      log "Wine: $("$WINE" --version 2>/dev/null) ($WINE)"
      return 0
    fi
    [ -n "${SMT5_WINE:-}" ] && die "SMT5_WINE=$SMT5_WINE Wine $major: en az Wine 11 gerekli"
    log "Wine $major eski, Wine $WINE_PIN indirilecek"
  fi
  local tmp; tmp="$(mktemp -d)"
  log "Wine $WINE_PIN indiriliyor ($WINE_DESC)"
  curl -fL --retry 3 --progress-bar -o "$tmp/wine.tar.xz" "$WINE_URL" || { rm -rf "$tmp"; die "Wine indirilemedi"; }
  [ "$(sha256 "$tmp/wine.tar.xz")" = "$WINE_SHA256" ] || { rm -rf "$tmp"; die "Wine paketinin SHA-256 özeti tutmuyor"; }
  rm -rf "$WINE_HOME.new" && mkdir -p "$WINE_HOME.new"
  tar -xJf "$tmp/wine.tar.xz" -C "$WINE_HOME.new" || { rm -rf "$tmp" "$WINE_HOME.new"; die "Wine paketi açılamadı"; }
  rm -rf "$tmp" "$WINE_HOME.old"
  [ -d "$WINE_HOME" ] && mv "$WINE_HOME" "$WINE_HOME.old"
  mv "$WINE_HOME.new" "$WINE_HOME"
  rm -rf "$WINE_HOME.old"
  resolve_wine || die "indirilen pakette wine bulunamadı"
  log "Wine hazır: $("$WINE" --version 2>/dev/null)"
}

ensure_prefix() {
  if [ ! -d "$PREFIX" ] && [ -f "$PROBE_PREFIX/system.reg" ]; then
    log "Deneme prefix'i (native-probe) yeniden kullanılıyor"
    mv "$PROBE_PREFIX" "$PREFIX" || die "native-probe taşınamadı"
  fi
  mkdir -p "$PREFIX"
  if [ ! -f "$PREFIX/system.reg" ]; then
    log "Wine prefix oluşturuluyor"
    "$WINE" wineboot --init >/dev/null 2>&1 || true
    "$WINESERVER" -w >/dev/null 2>&1 || true
    [ -f "$PREFIX/system.reg" ] || die "wineboot prefix oluşturamadı"
  fi
}

ensure_windows_python() {
  local tmp req_hash marker="$PYDIR/.smt5-requirements.sha256"
  if [ ! -f "$PYDIR/python.exe" ]; then
    tmp="$(mktemp -d)"
    log "Windows Python $PY_VER indiriliyor (NuGet paketi, kurulum programı yok)"
    curl -fsSL --retry 3 -o "$tmp/python.nupkg" "https://www.nuget.org/api/v2/package/python/$PY_VER" || { rm -rf "$tmp"; die "Python indirilemedi"; }
    mkdir -p "$tmp/x" "$PYDIR"
    unzip -q "$tmp/python.nupkg" 'tools/*' -d "$tmp/x" || { rm -rf "$tmp"; die "NuGet paketi açılamadı"; }
    cp -R "$tmp/x/tools/." "$PYDIR/"
    rm -rf "$tmp"
    "$WINE" "$WINPY" -m ensurepip --default-pip >/dev/null 2>&1 || die "pip kurulamadı"
  fi
  req_hash="$(sha256 "$SRC/config/requirements-wine.txt")"
  if [ "$(cat "$marker" 2>/dev/null)" != "$req_hash" ]; then
    log "Windows Python paketleri kuruluyor (MetaTrader5, siliconmetatrader5)"
    cp "$SRC/config/requirements-wine.txt" "$PYDIR/requirements-wine.txt"
    "$WINE" "$WINPY" -m pip install --no-cache-dir --disable-pip-version-check -q \
      -r 'C:\Program Files\Python313\requirements-wine.txt' || die "pip install başarısız"
    echo "$req_hash" >"$marker"
  fi
  "$WINE" "$WINPY" -c "import MetaTrader5, siliconmetatrader5.server" >/dev/null 2>&1 \
    || die "Windows Python paketleri yüklenemiyor (MetaTrader5 / siliconmetatrader5)"
}

safe_docker() { if [ -n "$SAFE_CONTEXT" ]; then docker --context "$SAFE_CONTEXT" "$@"; else docker "$@"; fi; }

import_from_safe() {  # copy MT5 (login, history included) from the Güvenli / Docker mode container
  command -v docker >/dev/null 2>&1 || return 1
  safe_docker inspect siliconmt5 >/dev/null 2>&1 || return 1
  log "MT5, Docker modundaki container'dan kopyalanıyor (broker girişi ve geçmiş dahil)"
  mkdir -p "$PREFIX/drive_c/Program Files"
  safe_docker cp "siliconmt5:/config/.wine/drive_c/Program Files/MetaTrader 5" - \
    | tar -x -C "$PREFIX/drive_c/Program Files"
  [ -f "$MT5_EXE" ]
}

ensure_mt5() {
  [ -f "$MT5_EXE" ] && return 0
  if [ "${SMT5_NATIVE_NO_IMPORT:-0}" != "1" ] && import_from_safe; then
    log "MT5 kopyalandı"
    return 0
  fi
  # Not copied from MetaQuotes' own Mac app: its terminal stops under this Wine
  # with "A debugger has been found running in your system".
  log "MetaTrader 5 kurulum programı çalıştırılıyor (birkaç dakika)"
  curl -fsSL --retry 3 -o "$PREFIX/drive_c/mt5setup.exe" "$MT5_URL" || die "mt5setup.exe indirilemedi"
  "$WINE" 'C:\mt5setup.exe' /auto >/dev/null 2>&1 &

  for _ in $(seq 1 90); do [ -f "$MT5_EXE" ] && break; sleep 10; done
  # let the installer finish copying, then close it and the terminal it starts
  sleep 20
  "$WINE" taskkill /IM mt5setup.exe /F >/dev/null 2>&1 || true
  "$WINE" taskkill /IM terminal64.exe >/dev/null 2>&1 || true
  sleep 5
  "$WINESERVER" -k >/dev/null 2>&1 || true
  rm -f "$PREFIX/drive_c/mt5setup.exe"
  [ -f "$MT5_EXE" ] || die "MT5 kurulamadı (terminal64.exe yok)"
  log "MT5 kuruldu"
}

ensure_venv() {
  local hp req_hash marker="$VENV/.smt5-requirements.sha256"
  if [ ! -x "$VENV/bin/python" ]; then
    if is_mac; then hp="$(host_python)" || die "Python 3.9+ gerekli: xcode-select --install (ya da brew install python)"
    else hp="$(host_python)" || die "Python 3.9+ ve venv gerekli: sudo apt install python3 python3-venv"; fi
    log "Durum API'si için Python ortamı oluşturuluyor ($("$hp" --version 2>&1))"
    "$hp" -m venv "$VENV" || die "python venv oluşturulamadı"
  fi
  req_hash="$(sha256 "$SRC/config/requirements-host.txt")"
  if [ "$(cat "$marker" 2>/dev/null)" != "$req_hash" ]; then
    "$VENV/bin/python" -m pip install --disable-pip-version-check -q -r "$SRC/config/requirements-host.txt" \
      || die "durum API'si paketleri kurulamadı"
    echo "$req_hash" >"$marker"
  fi
}

# The agent runs from a copy in Application Support: a background process
# reading ~/Documents would trigger macOS privacy prompts.
sync_runtime() {
  [ "$SRC" = "$RT" ] && return 0
  mkdir -p "$RT.new/macos" "$RT.new/status" "$RT.new/config"
  cp "$SRC/macos/native.sh" "$RT.new/macos/"
  cp "$SRC"/status/smt5_status.py "$SRC"/status/datatest.py "$SRC"/status/supertrend_strategy.py "$RT.new/status/"
  cp "$SRC"/config/mt5cfg.ini "$SRC"/config/requirements-*.txt "$RT.new/config/"
  rm -rf "$RT" && mv "$RT.new" "$RT"
  chmod 0755 "$RT/macos/native.sh"
  # settings for the agent (API token included): readable by this user only
  mkdir -p "$STATE" && chmod 0700 "$STATE"
  ( umask 077; grep -E '^SMT5_[A-Z0-9_]+=' "${SMT5_ENV_FILE:-$SRC/.env}" >"$ENV_COPY.tmp" 2>/dev/null || : >"$ENV_COPY.tmp" )
  mv -f "$ENV_COPY.tmp" "$ENV_COPY"
}

# Linux without a display: the installers (wineboot, mt5setup.exe) still need
# an X server, so the install runs on a short-lived Xvfb.
temp_display() {
  is_mac && return 0
  [ -n "${DISPLAY:-}" ] && return 0
  command -v Xvfb >/dev/null 2>&1 || { log "uyarı: ekran yok ve Xvfb kurulu değil; MT5 kurulum programı açılamayabilir (sudo apt install xvfb)"; return 0; }
  local n=111
  while [ -e "/tmp/.X11-unix/X$n" ] || [ -e "/tmp/.X$n-lock" ]; do n=$((n + 1)); done
  Xvfb ":$n" -screen 0 1366x768x24 -nolisten tcp >/dev/null 2>&1 &
  TEMP_XVFB=$!
  export DISPLAY=":$n"
  trap 'kill "$TEMP_XVFB" 2>/dev/null' EXIT
  for _ in 1 2 3 4 5 6 7 8 9 10; do [ -e "/tmp/.X11-unix/X$n" ] && break; sleep 0.5; done
  log "kurulum için geçici ekran: $DISPLAY (Xvfb)"
}

cmd_install() {
  need_macos
  COMPONENT=install
  mkdir -p "$BASE" "$STATE" "$LOGS"
  chmod 0700 "$STATE"
  if is_mac && ! /usr/bin/pgrep -q oahd 2>/dev/null && ! arch -x86_64 /usr/bin/true 2>/dev/null; then
    die "Rosetta 2 kurulu değil: softwareupdate --install-rosetta --agree-to-license"
  fi
  ensure_wine
  wine_env
  temp_display
  ensure_prefix
  ensure_windows_python
  ensure_mt5
  "$WINESERVER" -w >/dev/null 2>&1 || true
  ensure_venv
  sync_runtime
  # the temporary display must not leak into `start` (it would look like a desktop)
  if [ -n "${TEMP_XVFB:-}" ]; then kill "$TEMP_XVFB" 2>/dev/null; unset DISPLAY TEMP_XVFB; trap - EXIT; fi
  log "Kurulum tamam: $MT5_DIR"
}

# ------------------------------------------------------------ start/stop ---
write_plist() {
  local esc_script esc_logs
  esc_script="$(printf '%s' "$RT/macos/native.sh" | sed 's/&/\&amp;/g; s/</\&lt;/g')"
  esc_logs="$(printf '%s' "$LOGS/agent.log" | sed 's/&/\&amp;/g; s/</\&lt;/g')"
  cat >"$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array><string>/bin/bash</string><string>$esc_script</string><string>supervise</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
  <key>ProcessType</key><string>Interactive</string>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ExitTimeOut</key><integer>60</integer>
  <key>StandardOutPath</key><string>$esc_logs</string>
  <key>StandardErrorPath</key><string>$esc_logs</string>
</dict>
</plist>
EOF
}

cmd_start() {
  need_macos
  if ! installed; then cmd_install; else sync_runtime; fi
  load_env_file "$ENV_COPY"; ports
  if agent_loaded; then
    log "zaten çalışıyor (pid $(agent_pid))"
    return 0
  fi
  local p
  for p in "$BRIDGE_PORT" "$STATUS_PORT"; do
    port_open "$p" && die "127.0.0.1:$p kullanımda (Güvenli mod çalışıyor olabilir: smt5 mode safe && smt5 down)"
  done
  mkdir -p "$LOGS"
  if is_mac; then
    write_plist
    launchctl bootstrap "gui/$(id -u)" "$PLIST" || die "launchctl bootstrap başarısız"
    log "başlatıldı: MT5 penceresi birazdan açılır (durum: native.sh status)"
    return 0
  fi
  # Linux: window or headless is decided HERE, from the shell that ran start.
  # The systemd user manager keeps the desktop session's DISPLAY and
  # WAYLAND_DISPLAY even for an SSH login, so deciding inside the job would put
  # MT5 on the server's own (maybe locked) screen instead of noVNC.
  local envs=() v dmode
  dmode="$(display_mode)"
  envs+=("--setenv=SMT5_NATIVE_DISPLAY=$dmode")
  for v in XDG_RUNTIME_DIR DBUS_SESSION_BUS_ADDRESS SMT5_ENV_FILE; do
    [ -n "${!v:-}" ] && envs+=("--setenv=$v=${!v}")
  done
  if [ "$dmode" = window ]; then
    for v in DISPLAY WAYLAND_DISPLAY XAUTHORITY; do
      [ -n "${!v:-}" ] && envs+=("--setenv=$v=${!v}")
    done
  fi  # headless: start_display drops whatever the user manager itself carries
  if has_user_systemd; then
    systemctl --user reset-failed "$UNIT" >/dev/null 2>&1 || true
    systemd-run --user --quiet --collect --unit="$UNIT" -p KillMode=mixed -p TimeoutStopSec=60 \
      ${envs[@]+"${envs[@]}"} /bin/bash "$RT/macos/native.sh" supervise || die "systemd-run başarısız"
  else
    ( umask 077; setsid nohup /bin/bash "$RT/macos/native.sh" supervise >>"$LOGS/agent.log" 2>&1 </dev/null & echo $! >"$STATE/agent.pid" )
  fi
  log "başlatıldı (durum: native.sh status)"
}

cmd_stop() {
  need_macos
  if agent_loaded; then
    log "durduruluyor (MT5 kapanırken bekleniyor)"
    # who asked: shows up in supervisor.log next to the shutdown
    mkdir -p "$LOGS"
    printf '%s [stop] requested by pid %s, parent %s: %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$$" "$PPID" \
      "$(ps -o command= -p "$PPID" 2>/dev/null | cut -c1-160)" >>"$LOGS/supervisor.log"
    if is_mac; then launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
    elif has_user_systemd && systemctl --user is-active --quiet "$UNIT" 2>/dev/null; then systemctl --user stop "$UNIT" 2>/dev/null || true
    else kill -TERM "$(cat "$STATE/agent.pid" 2>/dev/null)" 2>/dev/null || true; fi
    for _ in $(seq 1 70); do agent_loaded || break; sleep 1; done
  fi
  rm -f "$STATE/agent.pid"
  stop_display
  # leftovers (crash, `run` killed hard): this prefix's Wine only
  if resolve_wine; then wine_env; "$WINESERVER" -k >/dev/null 2>&1 || true; fi
  pkill -f "$RT/status/smt5_status.py" 2>/dev/null || true
  rm -f "$STATE/supervisor.pid"
  log "durdu"
}

cmd_status() {
  local json=0 loaded=false inst=false wv="" phase="" detail="" pid=""
  [ "${1:-}" = "--json" ] && json=1
  agent_loaded && loaded=true && pid="$(agent_pid)"
  installed && inst=true
  resolve_wine && wv="$("$WINE" --version 2>/dev/null)"
  if [ -f "$STATE/state.json" ]; then
    phase="$(py -c 'import json,sys; print(json.load(open(sys.argv[1])).get("phase",""))' "$STATE/state.json" 2>/dev/null)"
    detail="$(py -c 'import json,sys; print(json.load(open(sys.argv[1])).get("detail",""))' "$STATE/state.json" 2>/dev/null)"
  fi
  if [ "$json" = 1 ]; then
    py - "$inst" "$loaded" "$pid" "$wv" "$phase" "$detail" "$BASE" <<'PY'
import json, sys
inst, loaded, pid, wv, phase, detail, base = sys.argv[1:8]
print(json.dumps({"installed": inst == "true", "running": loaded == "true", "pid": int(pid) if pid else None,
                  "wine": wv or None, "phase": phase or None, "detail": detail, "home": base}))
PY
  else
    echo "kurulu:   $inst ($BASE)"
    echo "wine:     ${wv:-yok}"
    echo "çalışıyor: $loaded${pid:+ (pid $pid)}"
    [ -n "$phase" ] && echo "faz:      $phase${detail:+ — $detail}"
  fi
}

# Copy MT5 from the Güvenli mode container again (e.g. after logging in to
# another broker there). The current MT5 folder is kept as a dated backup.
cmd_import() {
  need_macos
  installed || { cmd_install; return; }
  local was=0 bak=""
  agent_loaded && was=1 && cmd_stop
  wine_env
  if [ -d "$MT5_DIR" ]; then
    bak="$MT5_DIR.bak-$(date +%Y%m%d-%H%M%S)"
    mv "$MT5_DIR" "$bak" || die "mevcut MT5 klasörü yedeklenemedi"
    log "mevcut MT5 yedeklendi: $(basename "$bak")"
  fi
  if ! import_from_safe; then
    rm -rf "$MT5_DIR"
    [ -n "$bak" ] && mv "$bak" "$MT5_DIR"
    die "Güvenli moddaki container'a ulaşılamadı ($SAFE_CONTEXT): smt5 mode safe && smt5 vm start, sonra tekrar"
  fi
  rm -f "$STATE/mt5cfg.sha256"
  log "MT5 kopyalandı"
  [ "$was" = 1 ] && cmd_start
  return 0
}

cmd_reset() {  # MT5 data (prefix) only; Wine download and venv stay
  cmd_stop
  rm -rf "$PREFIX" "$STATE"
  log "Wine prefix silindi (MT5, broker girişi, geçmiş). Başlat yeniden kurar."
}

cmd_reset_all() {
  cmd_stop
  rm -rf "$PREFIX" "$PROBE_PREFIX" "$STATE" "$VENV" "$WINE_HOME" "$WINE_HOME.new" "$WINE_HOME.old" "$LOGS" "$RT"
  log "yerel modun tüm dosyaları silindi ($BASE)"
}

cmd_uninstall() {
  cmd_reset_all
  rm -rf "$BASE"
}

# ------------------------------------------------------------- supervisor ---
set_phase() {
  local phase="$1" detail="${2:-}"
  [ "$phase|$detail" = "${_LAST_PHASE:-}" ] && return 0
  _LAST_PHASE="$phase|$detail"
  py - "$phase" "$detail" "$STATE/state.json" <<'PY'
import json, os, sys, time
phase, detail, path = sys.argv[1:4]
with open(path + ".tmp", "w") as fh:
    json.dump({"phase": phase, "detail": detail, "updated": time.time()}, fh)
os.replace(path + ".tmp", path)
PY
  log "phase=$phase${detail:+ ($detail)}"
}

rotate_logs() {
  local max_mb="${SMT5_LOG_MAX_MB:-10}" max f size
  case "$max_mb" in ''|*[!0-9]*) max_mb=10 ;; esac
  [ "$max_mb" -ge 1 ] || max_mb=1
  max=$((max_mb * 1024 * 1024))
  for f in "$LOGS"/*.log; do
    [ -f "$f" ] || continue
    size="$(file_bytes "$f")"
    if [ "$size" -gt "$max" ]; then
      tail -c "$max" "$f" >"$f.1.tmp" 2>/dev/null && mv -f "$f.1.tmp" "$f.1"
      : >"$f"
      log "rotated $(basename "$f") ($((size / 1048576)) MB > ${max_mb} MB)"
    fi
  done
}

# mt5cfg.ini from the template + .env (MaxBars, Algo Trading, DLL imports);
# rewritten only when the result changes, so changes made in MT5 survive restarts.
sync_mt5cfg() {
  local tmp new old
  tmp="$(mktemp)"
  py - "$RT/config/mt5cfg.ini" "$tmp" <<'PY'
import os, sys
src, dst = sys.argv[1:3]
def b(v):
    v = (v or "").strip().lower()
    return "1" if v in ("1", "true", "yes", "on") else "0" if v in ("0", "false", "no", "off") else None
want = {}
mb = os.environ.get("SMT5_MAXBARS", "")
if mb.isdigit():
    want["MaxBars"] = mb
algo = b(os.environ.get("SMT5_ALGO_TRADING"))
if algo is not None:
    want["Enabled"] = want["AllowLiveTrading"] = algo
dll = b(os.environ.get("SMT5_DLL_IMPORT"))
if dll is not None:
    want["AllowDllImport"] = dll
out, done = [], set()
for line in open(src, encoding="utf-8").read().splitlines():
    k = line.split("=", 1)[0].strip()
    if "=" in line and not line.lstrip().startswith(";") and k in want and k not in done:
        line = f"{k}={want[k]}"
        done.add(k)
    out.append(line)
open(dst, "w", encoding="utf-8").write("\n".join(out) + "\n")
PY
  new="$(sha256 "$tmp")"
  old="$(cat "$STATE/mt5cfg.sha256" 2>/dev/null)"
  if [ ! -f "$MT5_DIR/mt5cfg.ini" ] || [ "$new" != "$old" ]; then
    cp "$tmp" "$MT5_DIR/mt5cfg.ini"
    echo "$new" >"$STATE/mt5cfg.sha256"
    log "mt5cfg.ini synchronized ($(grep -E '^(MaxBars|Enabled|AllowLiveTrading|AllowDllImport)=' "$tmp" | tr -d '\r' | tr '\n' ' '))"
  fi
  rm -f "$tmp"
}

pid_alive() { [ -n "${1:-}" ] && kill -0 "$1" 2>/dev/null; }
read_pid() { cat "$STATE/$1.pid" 2>/dev/null; }
# The PID we started is the terminal (wine execs in place; ps then shows
# "terminal64.exe /portable /config:mt5cfg.ini"). Fallback: that exact command
# line (MetaQuotes' own Mac app starts the terminal differently).
mt5_running() {
  pid_alive "$(read_pid mt5)" && return 0
  local pid; pid="$(pgrep -f '^terminal64\.exe /portable /config:mt5cfg\.ini' 2>/dev/null | head -1)"
  [ -n "$pid" ] && echo "$pid" >"$STATE/mt5.pid"
}

start_status() {
  pid_alive "$(read_pid status)" && return 0
  ( cd "$RT/status" && exec env \
      SMT5_STATUS_LISTEN=127.0.0.1 SMT5_STATUS_PORT_INTERNAL="$STATUS_PORT" \
      SMT5_BRIDGE_PORT_INTERNAL="$BRIDGE_PORT" SMT5_STATE_DIR="$STATE" SMT5_LOG_DIR="$LOGS" \
      SMT5_DISPLAY="${DISPLAY_KIND:-native}" SMT5_WINE_FLAVOR=native WINE_VERSION="$WINE_VERSION" \
      SMT5_CPU_TRANSLATION="$(is_mac && echo rosetta2 || echo none)" SMT5_STARTED_AT="$STARTED_AT" \
      "$VENV/bin/python" smt5_status.py ) >>"$LOGS/status.log" 2>&1 &
  echo $! >"$STATE/status.pid"
}

start_mt5() {
  MT5_T0="$(date +%s)"
  set_phase starting_terminal
  sync_mt5cfg
  [ "${DISPLAY_KIND:-}" = novnc ] && set_wine_dpi
  ( cd "$MT5_DIR" && exec "$WINE" terminal64.exe /portable /config:mt5cfg.ini ) >>"$LOGS/mt5.log" 2>&1 &
  echo $! >"$STATE/mt5.pid"
  [ "${DISPLAY_KIND:-}" = novnc ] && { maximize_mt5 </dev/null >>"$LOGS/display.log" 2>&1 & }
  return 0
}

# Headless only: a fixed 96 DPI (Windows' 100 %). Otherwise Wine derives the
# DPI from the virtual screen and MT5 comes out oversized in noVNC.
# SMT5_DPI (72–288) changes it, e.g. 120 = 125 %.
set_wine_dpi() {
  local dpi="${SMT5_DPI:-96}"
  case "$dpi" in ''|*[!0-9]*) dpi=96 ;; esac
  { [ "$dpi" -ge 72 ] && [ "$dpi" -le 288 ]; } || dpi=96
  "$WINE" reg add 'HKCU\Control Panel\Desktop' /v LogPixels /t REG_DWORD /d "$dpi" /f >/dev/null 2>&1 || true
  "$WINE" reg add 'HKLM\System\CurrentControlSet\Hardware Profiles\Current\Software\Fonts' /v LogPixels /t REG_DWORD /d "$dpi" /f >/dev/null 2>&1 || true
}

# Headless only: keep the MT5 main window maximized inside Wine (ShowWindow
# SW_MAXIMIZE, else SetWindowPos to the screen size) for its first 2 minutes.
# A window manager rule is not enough: Wine puts back the size MT5 saved.
MAXIMIZE_PY='import ctypes, ctypes.wintypes as w, time
u = ctypes.windll.user32
SW, SH = u.GetSystemMetrics(0), u.GetSystemMetrics(1)

def tops():
    out = []
    @ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    def each(h, _):
        if u.IsWindowVisible(h) and not u.GetWindow(h, 4):
            b = ctypes.create_unicode_buffer(256)
            u.GetClassNameW(h, b, 256)
            r = w.RECT()
            u.GetWindowRect(h, ctypes.byref(r))
            out.append((h, b.value, r.right - r.left, r.bottom - r.top))
        return True
    u.EnumWindows(each, 0)
    return out

def main_window():
    ws = tops()
    mt = [x for x in ws if x[1].startswith("MetaQuotes")]
    pick = max(mt or ws, key=lambda x: x[2] * x[3], default=None)
    return pick, ws

# MT5 restores its saved window placement a few seconds after it appears
# (and again after logging in), so keep it maximized for the first 2 minutes.
fixed, seen = 0, False
for i in range(60):
    pick, ws = main_window()
    if pick and not seen:
        seen = True
        print("smt5: screen %dx%d, windows %s" % (SW, SH, [(c, x, y) for _, c, x, y in ws]), flush=True)
    if pick:
        h = pick[0]
        r = w.RECT()
        u.GetWindowRect(h, ctypes.byref(r))
        if not u.IsZoomed(h) or (r.right - r.left) < SW - 8 or (r.bottom - r.top) < SH - 8:
            u.ShowWindow(h, 3)
            time.sleep(0.5)
            u.GetWindowRect(h, ctypes.byref(r))
            if (r.right - r.left) < SW - 8 or (r.bottom - r.top) < SH - 8:
                u.SetWindowPos(h, 0, 0, 0, SW, SH, 0x0004 | 0x0010)
                u.GetWindowRect(h, ctypes.byref(r))
            fixed += 1
            print("smt5: %s -> %dx%d at %d,%d" % (pick[1], r.right - r.left, r.bottom - r.top, r.left, r.top), flush=True)
    time.sleep(2)
print("smt5: done, %d fix(es)%s" % (fixed, "" if seen else ", MT5 window not found"), flush=True)'
maximize_mt5() {
  # as a file in the prefix: a -c argument would go through Windows
  # command-line quoting
  printf '%s\n' "$MAXIMIZE_PY" >"$PREFIX/drive_c/smt5_maximize.py"
  sleep 5
  ( cd "$PYDIR" && "$WINE" "$WINPY" 'C:\smt5_maximize.py' )
}

stop_mt5() {
  mt5_running || return 0
  log "stopping MT5"
  "$WINE" taskkill /IM terminal64.exe >/dev/null 2>&1 || true

  for _ in $(seq 1 30); do mt5_running || return 0; sleep 1; done
  log "MT5 did not close in 30s, forcing"
  "$WINE" taskkill /IM terminal64.exe /F >/dev/null 2>&1 || true
  sleep 2
  kill "$(read_pid mt5)" 2>/dev/null || true
}

start_bridge() {
  set_phase starting_bridge
  ( cd "$PYDIR" && exec "$WINE" "$WINPY" -m siliconmetatrader5.server --host 127.0.0.1 -p "$BRIDGE_PORT" ) \
    >>"$LOGS/bridge.log" 2>&1 &
  echo $! >"$STATE/bridge.pid"
  BRIDGE_STARTED_AT="$(date +%s)"
}

stop_bridge() {
  local pid; pid="$(read_pid bridge)"
  pid_alive "$pid" && kill "$pid" 2>/dev/null
  sleep 2
  pid_alive "$pid" && kill -9 "$pid" 2>/dev/null
  rm -f "$STATE/bridge.pid"
}

# ---------------------------------------------------------------- display ---
# macOS: MT5 is a normal window. Linux: the desktop's display when there is
# one, otherwise our own Xvfb + x11vnc (localhost) + noVNC (websockify) on
# 127.0.0.1:SMT5_DISPLAY_PORT. state/display records which ("window" / "novnc:<port>").
novnc_web() {
  local d
  for d in "${SMT5_NOVNC_WEB:-}" /usr/share/novnc /usr/share/webapps/novnc /opt/novnc; do
    [ -n "$d" ] && [ -f "$d/vnc.html" ] && { echo "$d"; return 0; }
  done
  return 1
}
display_mode() {
  if is_mac; then echo window; return 0; fi
  case "${SMT5_NATIVE_DISPLAY:-auto}" in
    window) echo window; return 0 ;;
    novnc|headless) echo novnc; return 0 ;;
  esac
  # A local X server (":0", or XWayland on a desktop session) shows the MT5
  # window on this machine's screen. No display, or one forwarded over SSH
  # ("localhost:10.0"), means headless: that window would vanish with the SSH
  # session, so run on Xvfb and serve it through noVNC instead.
  case "${DISPLAY:-}" in
    :*) echo window ;;
    *) echo novnc ;;
  esac
}
# x11vnc wants a screen width divisible by 4 ("VncViewer has problems"):
# 1366x768 → 1364x768. Bad values fall back to the default.
fit_res() {
  local r="${1:-1366x768}" w h
  w="${r%x*}"; h="${r#*x}"
  case "$w$h" in ''|*[!0-9]*) w=1366 h=768 ;; esac
  [ "$w" = "$r" ] && { w=1366; h=768; }
  echo "$((w / 4 * 4))x$h"
}
start_display() {
  DISPLAY_KIND="$(display_mode)"
  echo "$DISPLAY_KIND" >"$STATE/display"
  [ "$DISPLAY_KIND" = novnc ] || return 0
  local t web n=101 dport="${SMT5_DISPLAY_PORT:-6081}"
  valid_port "$dport" || dport=6081
  for t in Xvfb x11vnc websockify; do
    command -v "$t" >/dev/null 2>&1 || { set_phase error "ekransız çalışma için $t gerekli: sudo apt install xvfb x11vnc websockify novnc openbox"; return 1; }
  done
  web="$(novnc_web)" || { set_phase error "noVNC dosyaları yok: sudo apt install novnc"; return 1; }
  while [ -e "/tmp/.X11-unix/X$n" ] || [ -e "/tmp/.X$n-lock" ]; do n=$((n + 1)); done
  # Nothing from a desktop session may leak in: x11vnc exits when it sees
  # Wayland, and Wine would draw on the Wayland desktop instead of Xvfb.
  unset WAYLAND_DISPLAY XAUTHORITY
  export XDG_SESSION_TYPE=x11
  # no GPU behind Xvfb: draw in software from the start instead of Mesa
  # probing EGL/DRI and logging "libEGL warning: … dri2 screen" every time
  export LIBGL_ALWAYS_SOFTWARE=1 EGL_LOG_LEVEL=fatal MESA_LOG_LEVEL=error
  export DISPLAY=":$n"
  Xvfb "$DISPLAY" -screen 0 "$(fit_res "${SMT5_RESOLUTION:-}")x24" -dpi 96 -nolisten tcp </dev/null >>"$LOGS/display.log" 2>&1 &
  echo $! >"$STATE/xvfb.pid"
  for _ in 1 2 3 4 5 6 7 8 9 10; do [ -e "/tmp/.X11-unix/X$n" ] && break; sleep 0.5; done
  if command -v openbox >/dev/null 2>&1; then
    openbox_config
    openbox --config-file "$STATE/openbox-rc.xml" </dev/null >>"$LOGS/display.log" 2>&1 & echo $! >"$STATE/wm.pid"
  fi
  VNC_AUTH=(-nopw)
  if [ -n "${SMT5_WEB_PASSWORD:-}" ]; then
    ( umask 077; x11vnc -storepasswd "$SMT5_WEB_PASSWORD" "$STATE/vncpasswd" >/dev/null 2>&1 )
    VNC_AUTH=(-rfbauth "$STATE/vncpasswd")
  fi
  VNC_PORT=$((dport + 1000))                    # raw VNC: localhost only, for websockify
  start_x11vnc
  NOVNC_WEB="$web" NOVNC_PORT="$dport"
  websockify --web "$web" "127.0.0.1:$dport" "127.0.0.1:$VNC_PORT" </dev/null >>"$LOGS/display.log" 2>&1 &
  echo $! >"$STATE/websockify.pid"
  echo "novnc:$dport" >"$STATE/display"
  log "headless display $DISPLAY: noVNC on http://127.0.0.1:$dport/vnc.html"
}
openbox_config() {
  # No minimize button: in noVNC there is no taskbar to bring a minimized
  # window back. Maximize stays; maximize_mt5 fills the screen at start.
  cat >"$STATE/openbox-rc.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<openbox_config xmlns="http://openbox.org/3.4/rc">
  <desktops><number>1</number></desktops>
  <margins><top>0</top><bottom>0</bottom><left>0</left><right>0</right></margins>
  <theme><name>Clearlooks</name><titleLayout>NLMC</titleLayout></theme>
</openbox_config>
XML
}
start_x11vnc() {
  # Not -quiet: when x11vnc stops, the reason has to be in display.log. The
  # supervisor loop calls ensure_display, which starts it again if it exits.
  x11vnc -display "$DISPLAY" -forever -shared -localhost -rfbport "$VNC_PORT" "${VNC_AUTH[@]}" \
    -noxdamage </dev/null >>"$LOGS/display.log" 2>&1 &
  echo $! >"$STATE/x11vnc.pid"
}
ensure_display() {  # headless only: bring back a VNC server or proxy that exited
  [ "${DISPLAY_KIND:-}" = novnc ] || return 0
  local now; now="$(date +%s)"
  if ! pid_alive "$(read_pid x11vnc)" && [ "$now" -ge "${VNC_NEXT:-0}" ]; then
    VNC_RESTARTS=$((${VNC_RESTARTS:-0} + 1))
    # after 3 quick failures try once a minute, so the log does not fill up
    [ "$VNC_RESTARTS" -ge 3 ] && VNC_NEXT=$((now + 60))
    log "x11vnc kapandı (${VNC_RESTARTS}. kez), yeniden başlatılıyor; ayrıntı: logs/display.log"
    start_x11vnc
  fi
  if ! pid_alive "$(read_pid websockify)"; then
    log "websockify kapandı, yeniden başlatılıyor"
    websockify --web "$NOVNC_WEB" "127.0.0.1:$NOVNC_PORT" "127.0.0.1:$VNC_PORT" </dev/null >>"$LOGS/display.log" 2>&1 &
    echo $! >"$STATE/websockify.pid"
  fi
}
stop_display() {
  local c pid
  for c in websockify x11vnc wm xvfb; do
    pid="$(read_pid "$c")"
    pid_alive "$pid" && kill "$pid" 2>/dev/null
    rm -f "$STATE/$c.pid"
  done
  return 0
}

# The MetaTrader5 module in the bridge keeps an IPC channel to the terminal it
# attached to: after a terminal restart the bridge must restart too.
restart_terminal_and_bridge() {
  stop_bridge
  start_mt5
  nap 15
  start_bridge
  nap 10
}

nap() { sleep "$1" & wait $! 2>/dev/null; }   # interruptible by the TERM trap

shutdown() {
  log "received SIG${1:-TERM}"
  set_phase stopping
  stop_bridge
  stop_mt5
  "$WINESERVER" -k >/dev/null 2>&1 || true
  local pid; pid="$(read_pid status)"
  pid_alive "$pid" && kill "$pid" 2>/dev/null
  stop_display
  rm -f "$STATE"/*.pid "$STATE/maintenance.ready"
  set_phase stopped
  exit 0
}

# The user closed MT5: shut everything down like `stop`, then unload this
# launchd job (KeepAlive is off, so it stays down until Start).
user_closed() {
  trap - TERM INT HUP
  set_phase stopping "MT5 penceresi kapatıldı"
  stop_bridge
  "$WINESERVER" -k >/dev/null 2>&1 || true
  local pid; pid="$(read_pid status)"
  pid_alive "$pid" && kill "$pid" 2>/dev/null
  rm -f "$STATE"/*.pid "$STATE/maintenance.ready"
  stop_display
  set_phase stopped "MT5 penceresi kapatıldı"
  rm -f "$STATE/agent.pid"
  # launchd keeps a finished agent loaded: unload it. A systemd-run unit or a
  # detached process simply ends here.
  if is_mac; then exec launchctl bootout "gui/$(id -u)/$LABEL"; fi
  exit 0
}

cmd_supervise() {
  need_macos
  COMPONENT=supervisor
  mkdir -p "$STATE" "$LOGS"
  if [ -t 1 ]; then exec > >(tee -a "$LOGS/supervisor.log") 2>&1; else exec >>"$LOGS/supervisor.log" 2>&1; fi
  local other; other="$(read_pid supervisor)"
  if pid_alive "$other" && [ "$other" != "$$" ]; then
    log "another supervisor is already running (pid $other); exiting"
    exit 0
  fi
  echo $$ >"$STATE/supervisor.pid"
  load_env_file "$ENV_COPY"
  ports
  resolve_wine || { set_phase error "Wine bulunamadı: native.sh install"; exit 1; }
  wine_env
  WINE_VERSION="$("$WINE" --version 2>/dev/null)"
  STARTED_AT="$(date +%s)"
  BRIDGE_STARTED_AT=0
  rm -f "$STATE/maintenance.ready" "$STATE/maintenance.request"
  trap 'shutdown TERM' TERM
  trap 'shutdown INT' INT
  trap 'shutdown HUP' HUP
  log "supervisor started (pid $$, $WINE_VERSION)"
  start_display || { while :; do nap 3600; done; }

  set_phase preparing_prefix
  start_status
  if [ ! -f "$MT5_EXE" ] || [ ! -f "$PYDIR/python.exe" ]; then
    set_phase error "kurulum eksik: native.sh install"
    while :; do nap 3600; done
  fi
  start_mt5
  nap 15
  start_bridge

  # MT5 that closes within 90 s three times in a row is not restarted again
  # (an error dialog would come back forever) until a restart is requested.
  local misses=0 loops=0 quick=0 halted=0
  while :; do
    loops=$((loops + 1))
    [ $((loops % 12)) -eq 1 ] && rotate_logs
    pid_alive "$(read_pid status)" || { log "status API died, restarting it"; start_status; }
    ensure_display
    if [ -f "$STATE/maintenance.request" ]; then
      set_phase maintenance "$(head -c 200 "$STATE/maintenance.request" 2>/dev/null)"
      stop_bridge
      stop_mt5
      touch "$STATE/maintenance.ready"
      while [ -f "$STATE/maintenance.request" ]; do nap 1; done
      rm -f "$STATE/maintenance.ready"
      log "maintenance finished"
      restart_terminal_and_bridge
      continue
    fi
    if [ -f "$STATE/restart-mt5.request" ]; then
      rm -f "$STATE/restart-mt5.request"
      log "MT5 restart requested"
      halted=0 quick=0
      stop_mt5
      restart_terminal_and_bridge
      continue
    fi
    if [ -f "$STATE/restart-bridge.request" ]; then
      rm -f "$STATE/restart-bridge.request"
      log "bridge restart requested"
      stop_bridge
      start_bridge
      nap 10
      continue
    fi
    if [ "$halted" = 1 ]; then nap 5; continue; fi
    if ! mt5_running; then
      # Exit code 0 = the user closed the MT5 window (or File › Exit): stop the
      # whole runtime instead of reopening it (SMT5_MT5_ON_CLOSE=restart keeps
      # the old behaviour). A crash (non-zero) is restarted below.
      local code=0 mpid; mpid="$(read_pid mt5)"
      if [ -n "$mpid" ]; then wait "$mpid" 2>/dev/null; code=$?; else code=127; fi
      nap 5                                    # a LiveUpdate restart starts a new terminal
      if mt5_running; then continue; fi
      if [ "$code" = 0 ] && [ "${SMT5_MT5_ON_CLOSE:-stop}" != "restart" ]; then
        log "[stop] MT5 window closed by the user (exit 0): stopping the runtime"
        user_closed
      fi
      if [ $(( $(date +%s) - ${MT5_T0:-0} )) -lt 90 ]; then quick=$((quick + 1)); else quick=0; fi
      if [ "$quick" -ge 3 ]; then
        log "MT5 closed $quick times within 90 s of starting; not restarting it (see mt5.log and MT5's own logs)"
        stop_bridge
        halted=1
        set_phase error "MT5 açılır açılmaz kapanıyor (3 kez); Loglar › mt5"
        continue
      fi
      log "MT5 exited (code $code), restarting it together with the bridge"
      restart_terminal_and_bridge
      continue
    fi
    if port_open "$BRIDGE_PORT"; then
      misses=0
      set_phase running
    else
      misses=$((misses + 1))
      if [ "$misses" -ge 6 ] && [ $(( $(date +%s) - BRIDGE_STARTED_AT )) -ge 60 ]; then
        log "bridge not listening on :$BRIDGE_PORT, restarting"
        stop_bridge
        start_bridge
        misses=0
      fi
    fi
    nap 5
  done
}

# ------------------------------------------------------------------- main ---
case "${1:-}" in
  install) cmd_install ;;
  import) cmd_import ;;
  start) cmd_start ;;
  stop) cmd_stop ;;
  restart) cmd_stop; cmd_start ;;
  status) shift; cmd_status "${1:-}" ;;
  supervise) cmd_supervise ;;
  run)
    need_macos
    installed || cmd_install
    sync_runtime
    agent_loaded && die "agent çalışıyor: önce native.sh stop"
    cmd_supervise
    ;;
  reset) cmd_reset ;;
  reset-all) cmd_reset_all ;;
  uninstall) cmd_uninstall ;;
  *)
    awk 'NR > 1 && !/^#/ { exit } NR > 1 { sub(/^# ?/, ""); print }' "$0"
    exit 2
    ;;
esac
