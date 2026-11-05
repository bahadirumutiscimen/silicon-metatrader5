#!/bin/bash
# CPU translation helpers, sourced by build-runtime.sh (build time) and
# common.sh (run time). Keep this file small and stable: it is part of the
# image's slow build layer.

# How is this x86_64 container being executed?
#   none       real x86_64 CPU, or a full x86_64 VM (Colima/Lima --arch x86_64 --vm-type=qemu)
#   rosetta    Apple Rosetta user-mode translation (Docker Desktop / Colima vz+rosetta)
#   user-mode  other user-mode translation on an ARM kernel (qemu-user binfmt, FEX, box64)
# Wine crashes under Rosetta (AVX xsave assertion in wineboot) and is unreliable
# under qemu-user, so both are refused unless SMT5_ALLOW_USER_EMULATION=1.
detect_cpu_translation() {
  if grep -qi rosetta /proc/self/maps 2>/dev/null; then
    echo rosetta
  elif grep -qiE '^(CPU implementer|CPU architecture)' /proc/cpuinfo 2>/dev/null; then
    echo user-mode
  else
    echo none
  fi
}

translation_help() {
  cat <<'EOF'
MetaTrader 5 needs a real x86_64 CPU or a full x86_64 virtual machine.
This container is running under user-mode translation (Rosetta or qemu-user),
where Wine crashes. On a Mac use the Mac yerel mode (smt5 mode native) or a
separate QEMU x86_64 VM:

  colima start --profile x86 --arch x86_64 --vm-type=qemu --cpu 4 --memory 8 --disk 60

Linux ARM64 is not supported. To try anyway, set SMT5_ALLOW_USER_EMULATION=1.
EOF
}
