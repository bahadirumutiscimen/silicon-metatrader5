#!/usr/bin/env bash
# SiliconMetaTrader5 server preflight: what this machine has and lacks for
# the runtime. Prints one JSON object on stdout; changes nothing. The desktop
# app runs it over ssh (bash -s) before installing (Server mode).
set -u
R="$HOME/siliconmetatrader5"
U="${USER:-$(id -un)}"

has() { command -v "$1" >/dev/null 2>&1; }
b() { if "$@" >/dev/null 2>&1; then printf true; else printf false; fi; }
js() { # JSON string (or null when empty)
  if [ -z "$1" ]; then printf null; return; fi
  printf '"%s"' "$(printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr -d '\000-\037')"
}
num() { case "$1" in ''|*[!0-9]*) printf 0 ;; *) printf '%s' "$1" ;; esac; }

os_id="" os_pretty=""
if [ -r /etc/os-release ]; then
  os_id="$(. /etc/os-release 2>/dev/null; printf '%s' "${ID:-}")"
  os_pretty="$(. /etc/os-release 2>/dev/null; printf '%s' "${PRETTY_NAME:-}")"
fi
disk="$(df -Pm "$HOME" 2>/dev/null | awk 'NR==2 {print $4}')"
mem="$(awk '/^MemTotal:/ {print int($2 / 1024)}' /proc/meminfo 2>/dev/null)"
linger=false
[ "$(loginctl show-user "$U" -p Linger --value 2>/dev/null)" = yes ] && linger=true
version=""
[ -f "$R/VERSION" ] && version="$(head -n1 "$R/VERSION" 2>/dev/null)"

printf '{"arch":%s,"os_id":%s,"os_pretty":%s,"disk_free_mb":%s,"mem_mb":%s,' \
  "$(js "$(uname -m)")" "$(js "$os_id")" "$(js "$os_pretty")" "$(num "$disk")" "$(num "$mem")"
printf '"systemd_user":%s,"linger":%s,"apt":%s,"sudo_nopass":%s,"docker_access":%s,' \
  "$(b systemctl --user show-environment)" "$linger" "$(b has apt-get)" "$(b sudo -n true)" "$(b docker info)"
printf '"installed":%s,"version":%s,' "$(b test -d "$R/runtime")" "$(js "$version")"
printf '"tools":{"Xvfb":%s,"x11vnc":%s,"websockify":%s,"novnc":%s,"python3_venv":%s,"curl":%s,"xz":%s,"xclip":%s,"docker":%s,"compose":%s}}\n' \
  "$(b has Xvfb)" "$(b has x11vnc)" "$(b has websockify)" \
  "$(b test -f /usr/share/novnc/vnc.html)" \
  "$(b python3 -c 'import venv, ensurepip')" \
  "$(b has curl)" "$(b has xz)" "$(b has xclip)" "$(b has docker)" "$(b docker compose version)"
