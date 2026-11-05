#!/bin/bash
# Build-time: system packages, WineHQ, the noVNC display stack and the
# host-side venv used by the status API.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

pkgs=(
  ca-certificates curl gnupg cabextract unzip procps iproute2 tzdata
  xvfb xdotool python3 python3-venv
  # display: Xvfb + x11vnc + noVNC (websockify); xclip = clipboard sync with the desktop app
  x11vnc openbox tini python3-websockify x11-utils xclip
)

apt-get update
apt-get install -y --no-install-recommends "${pkgs[@]}"

# --- WineHQ (Debian bookworm) -------------------------------------------
install -d -m 0755 /etc/apt/keyrings
curl -fsSL --retry 3 https://dl.winehq.org/wine-builds/winehq.key \
  -o /etc/apt/keyrings/winehq-archive.key
curl -fsSL --retry 3 https://dl.winehq.org/wine-builds/debian/dists/bookworm/winehq-bookworm.sources \
  -o /etc/apt/sources.list.d/winehq-bookworm.sources
dpkg --add-architecture i386
apt-get update
if [ -n "${WINE_VERSION:-}" ]; then
  # Pin an exact build, e.g. WINE_VERSION=10.0.0.0~bookworm-1
  apt-get install -y --install-recommends \
    "winehq-stable=${WINE_VERSION}" "wine-stable=${WINE_VERSION}" \
    "wine-stable-amd64=${WINE_VERSION}" "wine-stable-i386=${WINE_VERSION}"
else
  apt-get install -y --install-recommends winehq-stable
fi
wine --version

# --- noVNC (pinned tag, never master) -----------------------------------
: "${NOVNC_VERSION:?NOVNC_VERSION must be set}"
curl -fsSL --retry 3 "https://github.com/novnc/noVNC/archive/refs/tags/v${NOVNC_VERSION}.tar.gz" \
  | tar -xz -C /opt
mv "/opt/noVNC-${NOVNC_VERSION}" /opt/novnc
ln -sf vnc.html /opt/novnc/index.html

# --- Host-side venv for the status API's bridge probe -------------------
python3 -m venv /opt/smt5-venv
/opt/smt5-venv/bin/pip install --no-cache-dir -r /siliconmt5/config/requirements-host.txt

apt-get clean
rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*
