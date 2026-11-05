# syntax=docker/dockerfile:1
#
# SiliconMetaTrader5 runtime: MetaTrader 5 + the Python bridge in Wine, with a web
# desktop (Xvfb + x11vnc + noVNC).
#
#   docker compose build          # or: docker build -t siliconmt5 .
#
# Ports inside the container: noVNC :3000, bridge :8001, status/data API :8002
# (compose publishes them on 127.0.0.1 only).
#
# The image is linux/amd64 only: MT5 is an x86_64 Windows binary. On Apple
# Silicon it runs in a full x86_64 QEMU VM (Colima profile `x86`) — or, faster,
# without Docker in the Mac yerel mode (runtime/macos/native.sh).

ARG BASE=debian:bookworm-slim

FROM ${BASE}

ARG PYTHON_VERSION=3.13.9
ARG SMT5_ALLOW_USER_EMULATION=0
ARG WINE_VERSION=
ARG NOVNC_VERSION=1.5.0

LABEL org.opencontainers.image.title="siliconmt5" \
      org.opencontainers.image.description="MetaTrader 5 + Python bridge runtime (Wine, noVNC)" \
      org.opencontainers.image.source="https://github.com/bahadirumutiscimen/silicon-metatrader5" \
      org.opencontainers.image.licenses="MIT"

ENV DEBIAN_FRONTEND=noninteractive \
    SMT5_DISPLAY=novnc \
    PYTHON_VERSION=${PYTHON_VERSION} \
    WINE_VERSION=${WINE_VERSION} \
    NOVNC_VERSION=${NOVNC_VERSION} \
    WINEPREFIX=/config/.wine \
    WINEDEBUG=-all \
    WIN_PYTHON="C:/Program Files/Python313/python.exe"

WORKDIR /siliconmt5

# 1) System packages, WineHQ, noVNC, host-side venv (rarely changes)
COPY config/requirements-host.txt /siliconmt5/config/requirements-host.txt
COPY scripts/install-system.sh /siliconmt5/scripts/install-system.sh
RUN bash /siliconmt5/scripts/install-system.sh

# 2) Bake the Wine prefix: Windows Python, VC++ runtime, bridge packages (slow).
#    Only the files this step needs, so script edits do not rebuild it.
COPY config/requirements-wine.txt config/vcrun-overrides.reg /siliconmt5/config/
COPY scripts/cpu-translation.sh scripts/build-runtime.sh /siliconmt5/scripts/
RUN bash /siliconmt5/scripts/build-runtime.sh

# 3) Runtime scripts, status API, MT5 config (changes often, cheap layers)
COPY config/ /siliconmt5/config/
COPY scripts/ /siliconmt5/scripts/
COPY status/ /siliconmt5/status/
RUN chmod 0755 /siliconmt5/scripts/*.sh

EXPOSE 3000 8001 8002
VOLUME ["/config"]
ENTRYPOINT ["/usr/bin/tini", "--", "/siliconmt5/scripts/entrypoint-novnc.sh"]
