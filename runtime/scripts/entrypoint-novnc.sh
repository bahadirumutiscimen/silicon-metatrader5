#!/bin/bash
# Entrypoint for the noVNC target: Xvfb + x11vnc (localhost only) + noVNC on
# :3000, the status API on :8002, then the supervisor in the foreground.
set -euo pipefail
SMT5_COMPONENT=novnc
# shellcheck source=common.sh
source /siliconmt5/scripts/common.sh

mkdir -p "$SMT5_STATE_DIR" "$SMT5_LOG_DIR"
export DISPLAY=:100

rm -f /tmp/.X100-lock /tmp/.X11-unix/X100
Xvfb :100 -screen 0 "${SMT5_RESOLUTION}x24" -dpi 96 -nolisten tcp >>"$SMT5_LOG_DIR/display.log" 2>&1 &
for _ in $(seq 1 50); do
  xdpyinfo >/dev/null 2>&1 && break
  sleep 0.2
done

if [ "${SMT5_WM:-0}" = "1" ]; then
  openbox >>"$SMT5_LOG_DIR/display.log" 2>&1 &
fi

vnc_auth=(-nopw)
if [ -n "${SMT5_WEB_PASSWORD:-}" ]; then
  x11vnc -storepasswd "$SMT5_WEB_PASSWORD" "$SMT5_STATE_DIR/vncpasswd" >/dev/null 2>&1
  chmod 600 "$SMT5_STATE_DIR/vncpasswd"
  vnc_auth=(-rfbauth "$SMT5_STATE_DIR/vncpasswd")
else
  log "WARNING: SMT5_WEB_PASSWORD is empty; the desktop is only protected by the 127.0.0.1 bind"
fi

# -localhost: raw VNC (5901) is reachable only by websockify inside the container.
x11vnc -display :100 -forever -shared -localhost -rfbport 5901 "${vnc_auth[@]}" \
  -skip_lockkeys -quiet -noprimary -nosetprimary >>"$SMT5_LOG_DIR/display.log" 2>&1 &
/opt/novnc/utils/novnc_proxy --vnc localhost:5901 --listen 3000 --web /opt/novnc \
  >>"$SMT5_LOG_DIR/display.log" 2>&1 &

/opt/smt5-venv/bin/python /siliconmt5/status/smt5_status.py >>"$SMT5_LOG_DIR/status.log" 2>&1 &

exec /siliconmt5/scripts/supervisor.sh
