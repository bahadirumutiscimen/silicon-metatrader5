#!/bin/bash
# Build-time: bake a Wine prefix with Windows Python, the VC++ runtime and the
# bridge packages into /opt/wineprefix-base. prepare-prefix.sh copies it into
# the persistent /config volume on first start.
set -euo pipefail

: "${PYTHON_VERSION:?PYTHON_VERSION must be set}"
: "${WIN_PYTHON:?WIN_PYTHON must be set}"

# shellcheck source=cpu-translation.sh
source /siliconmt5/scripts/cpu-translation.sh
export WINEPREFIX=/opt/wineprefix-base
export WINEDEBUG=-all

translation="$(detect_cpu_translation)"
echo "[build] cpu translation: $translation"
if [ "$translation" != "none" ] && [ "${SMT5_ALLOW_USER_EMULATION:-0}" != "1" ]; then
  echo "[build] ERROR: refusing to build under '$translation' translation." >&2
  translation_help >&2
  exit 2
fi

VCREDIST_X86_URL="${VCREDIST_X86_URL:-https://aka.ms/vs/16/release/vc_redist.x86.exe}"
VCREDIST_X64_URL="${VCREDIST_X64_URL:-https://aka.ms/vs/16/release/vc_redist.x64.exe}"
work=/tmp/smt5-build
mkdir -p "$WINEPREFIX" "$work"

Xvfb :99 -screen 0 1024x768x24 -nolisten tcp >"$work/xvfb.log" 2>&1 &
xvfb_pid=$!
cleanup() { kill "$xvfb_pid" 2>/dev/null || true; rm -rf "$work"; }
trap cleanup EXIT
export DISPLAY=:99
sleep 2

echo "[build] initializing wine prefix"
# wineboot may exit non-zero even when it worked, so judge by the result.
WINEDLLOVERRIDES="mscoree,mshtml=" wineboot --init || true
wineserver -w
if [ ! -s "$WINEPREFIX/system.reg" ] || [ ! -f "$WINEPREFIX/drive_c/windows/system32/kernel32.dll" ]; then
  echo "[build] ERROR: wineboot did not create a usable prefix" >&2
  translation_help >&2
  exit 3
fi
wine reg add 'HKCU\Software\Wine' /v Version /t REG_SZ /d win10 /f >/dev/null

echo "[build] installing Windows Python ${PYTHON_VERSION}"
curl -fsSL --retry 3 -o "$work/python.exe" \
  "https://www.python.org/ftp/python/${PYTHON_VERSION}/python-${PYTHON_VERSION}-amd64.exe"
if [ -n "${PYTHON_SHA256:-}" ]; then
  echo "${PYTHON_SHA256}  $work/python.exe" | sha256sum -c -
fi
# No tests, docs (60 MB) or Tcl/Tk/IDLE (~15 MB): the bridge needs none of them.
timeout 2700 wine "$work/python.exe" /quiet InstallAllUsers=1 PrependPath=1 \
  Include_test=0 Include_doc=0 Include_tcltk=0 \
  || echo "[build] python installer exited with $? (checking result)"
wineserver -w
python_exe="$WINEPREFIX/drive_c/Program Files/Python313/python.exe"
if [ ! -f "$python_exe" ]; then
  echo "[build] ERROR: Windows Python was not installed ($python_exe missing)" >&2
  exit 4
fi

# VC++ 2015-2022 runtime. The official installer hangs under Wine, so the DLLs
# are extracted by hand (fix carried over from silicon-metatrader5-linux).
echo "[build] installing VC++ runtime (manual extraction)"
for arch in x86 x64; do
  url_var="VCREDIST_${arch^^}_URL"
  mkdir -p "$work/vc-$arch" "$work/dll-$arch"
  curl -fsSL --retry 3 -o "$work/vc_redist.$arch.exe" "${!url_var}"
  cabextract -q -d "$work/vc-$arch" "$work/vc_redist.$arch.exe"
  # The inner cabinets have no .cab extension (a10, a11, ...): try them all.
  for cab in "$work/vc-$arch"/*; do
    cabextract -q -d "$work/dll-$arch" "$cab" >/dev/null 2>&1 || true
  done
  test -f "$work/dll-$arch/vcruntime140.dll" || { echo "[build] vcruntime140.dll missing for $arch" >&2; exit 1; }
done
cp "$work"/dll-x86/*.dll "$WINEPREFIX/drive_c/windows/syswow64/"
cp "$work"/dll-x64/*.dll "$WINEPREFIX/drive_c/windows/system32/"
wine regedit /S /siliconmt5/config/vcrun-overrides.reg
# Native ucrtbase must not be shadowed by the KnownDLLs entry (crealf fix).
wine reg delete 'HKLM\System\CurrentControlSet\Control\Session Manager\KnownDLLs' /v ucrtbase /f >/dev/null 2>&1 || true
wineserver -w

echo "[build] installing bridge packages into Windows Python"
wine "$WIN_PYTHON" -m pip install --no-cache-dir --disable-pip-version-check \
  -r 'Z:\siliconmt5\config\requirements-wine.txt'

echo "[build] validating"
wine "$WIN_PYTHON" -c "import MetaTrader5, rpyc, siliconmetatrader5.server; print('bridge packages ok')"
wineserver -w

echo "[build] trimming installer caches"
# Copies of the Python MSIs, only needed to uninstall/repair Python (~45 MB).
rm -rf "$WINEPREFIX/drive_c/ProgramData/Package Cache" "$WINEPREFIX/drive_c/windows/Installer"
find "$WINEPREFIX/drive_c/users" -path '*/Temp/*' -delete 2>/dev/null || true
rm -rf "$WINEPREFIX/drive_c/windows/temp/"* 2>/dev/null || true
du -sh "$WINEPREFIX" | sed 's/^/[build] prefix size: /'

touch "$WINEPREFIX/.runtime_baked"
echo "[build] done"
