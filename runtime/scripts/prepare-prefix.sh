#!/bin/bash
# Seed the persistent Wine prefix (/config/.wine) from the baked one on first
# start, then make sure it is initialised for the current user.
#
# Seeding is atomic: the copy goes to a temporary directory and is renamed
# only when complete, so a container stop / host sleep in the middle of the
# (slow, under QEMU) copy can never leave a half-copied prefix behind.
set -euo pipefail
SMT5_COMPONENT=prefix
# shellcheck source=common.sh
source /siliconmt5/scripts/common.sh

BASE_PREFIX=/opt/wineprefix-base
SEED_MARKER="$WINEPREFIX/.seed_complete"
mkdir -p "$(dirname "$WINEPREFIX")" "$SMT5_STATE_DIR" "$SMT5_LOG_DIR"

if [ ! -f "$SEED_MARKER" ] && [ ! -f "$MT5_EXE" ]; then
  tmp="${WINEPREFIX}.seeding"
  rm -rf "$tmp" "$WINEPREFIX"   # leftovers of an interrupted copy

  # The copy plus MT5 and its history need room; fail loudly instead of
  # filling the disk (a full disk also prevents writing the error state).
  base_mb=$(( $(du -sk "$BASE_PREFIX" | cut -f1) / 1024 ))
  need_mb=$(( base_mb + 2048 ))
  have_mb="$(free_mb "$(dirname "$WINEPREFIX")")"
  if [ "${have_mb:-0}" -lt "$need_mb" ]; then
    set_phase error "not enough disk space for the Wine prefix: need ${need_mb} MB, free ${have_mb} MB (enlarge the VM disk or prune Docker)"
    exit 1
  fi

  set_phase preparing_prefix "copying Wine prefix (first start, can take several minutes)"
  log "seeding persistent prefix from baked runtime (${base_mb} MB, ${have_mb} MB free)"
  started="$(date +%s)"
  if ! cp -a "$BASE_PREFIX" "$tmp"; then
    rm -rf "$tmp"   # give the space back so the error can be recorded
    set_phase error "copying the Wine prefix failed ($(free_mb "$(dirname "$WINEPREFIX")") MB free) — see ./smt5 logs supervisor"
    exit 1
  fi
  touch "$tmp/.seed_complete"
  mv "$tmp" "$WINEPREFIX"
  log "prefix seeded in $(( $(date +%s) - started ))s"
elif [ ! -f "$SEED_MARKER" ]; then
  # Prefix from an earlier build that already has MT5 installed: keep it.
  touch "$SEED_MARKER"
fi

# Wine refuses a prefix that is not owned by the current user.
if [ "$(stat -c %u "$WINEPREFIX")" != "$(id -u)" ]; then
  log "fixing prefix ownership for uid $(id -u)"
  chown -R "$(id -u):$(id -g)" "$WINEPREFIX"
fi
chmod 700 "$WINEPREFIX" 2>/dev/null || true

if [ ! -f "$WINEPREFIX/.prefix_ready" ]; then
  set_phase preparing_prefix "initialising Wine prefix"
  log "initialising prefix"
  started="$(date +%s)"
  WINEDLLOVERRIDES="mscoree,mshtml=" timeout 900 wineboot --init >/dev/null 2>&1 || log "wineboot exited with $?"
  timeout 120 wineserver -w || true
  touch "$WINEPREFIX/.prefix_ready"
  log "prefix initialised in $(( $(date +%s) - started ))s"
fi

# Idempotent: keep native ucrtbase usable (crealf fix).
timeout 120 wine reg delete 'HKLM\System\CurrentControlSet\Control\Session Manager\KnownDLLs' \
  /v ucrtbase /f >/dev/null 2>&1 || true
log "prefix ready"
