#!/usr/bin/env bash
# SiliconMetaTrader5 test on a Linux x86_64 machine with Docker Engine.
#
#   bash packaging/linux/test-linux.sh               # Docker mode: build the image, start, wait for MT5
#   bash packaging/linux/test-linux.sh --native      # Linux native mode: Wine 11 without Docker (headless: Xvfb + noVNC)
#   bash packaging/linux/test-linux.sh --after-login  # data test, Supertrend, history, bench
#   bash packaging/linux/test-linux.sh --cli         # also build and run the smt5 CLI (needs cargo)
#
# Everything is written to smt5-linux-test.log (paste that file back). The
# noVNC password and the API token stay in runtime/.env and are never printed;
# account numbers are masked. Only the compose project "siliconmt5" is touched.
set -u
cd "$(dirname "$0")/../.." || exit 1
RT="$PWD/runtime" LOG="$PWD/smt5-linux-test.log"
AFTER=0 CLI=0 NATIVE=0
for a in "$@"; do case "$a" in --after-login) AFTER=1 ;; --cli) CLI=1 ;; --native) NATIVE=1 ;; *) echo "unknown option $a"; exit 2 ;; esac; done
exec > >(tee -a "$LOG") 2>&1
say() { printf '\n== %s\n' "$*"; }
ok() { printf '   ok    %s\n' "$*"; }
bad() { printf '   FAIL  %s\n' "$*"; FAILS=$((FAILS + 1)); }
FAILS=0
dc() { docker compose -f "$RT/compose.yaml" --env-file "$RT/.env" "$@"; }
api() { curl -s --max-time "${2:-30}" "http://127.0.0.1:${STATUS_PORT}$1"; }
pyj() { python3 -c "import json,sys; d=json.load(sys.stdin); $1" 2>/dev/null || echo "   (could not parse the answer)"; }

say "SiliconMetaTrader5 Linux test · $(date '+%Y-%m-%d %H:%M:%S') · $( [ "$AFTER" = 1 ] && echo after-login || echo setup)"
say "System"
uname -srm
. /etc/os-release 2>/dev/null && echo "$PRETTY_NAME"
echo "CPU: $(nproc) cores · RAM: $(free -g 2>/dev/null | awk '/Mem:/ {print $2 " GB"}') · free disk here: $(df -h . | awk 'NR==2 {print $4}')"
[ "$(uname -m)" = "x86_64" ] && ok "x86_64" || bad "not x86_64 ($(uname -m)): Linux ARM64 is not supported"
command -v python3 >/dev/null && ok "python3 $(python3 -V 2>&1 | cut -d' ' -f2)" || bad "python3 missing (only used to read JSON here)"

say "Docker"
# Docker is only required for Docker mode. Native mode and --after-login (in
# native mode) run without it, so a missing Docker is just a note there.
NEED_DOCKER=1
{ [ "$NATIVE" = 1 ] || [ "$AFTER" = 1 ]; } && NEED_DOCKER=0
HAVE_DOCKER=0
if ! command -v docker >/dev/null; then
  if [ "$NEED_DOCKER" = 1 ]; then bad "docker missing: curl -fsSL https://get.docker.com | sh   (or use --native: no Docker needed)"; exit 1; fi
  echo "   not installed (not needed for native mode)"
elif ! docker info >/dev/null 2>&1; then
  if [ "$NEED_DOCKER" = 1 ]; then bad "engine not reachable: sudo usermod -aG docker \$USER, then log out and in"; exit 1; fi
  echo "   installed but not reachable as $(id -un) (not needed for native mode)"
elif ! docker compose version >/dev/null 2>&1; then
  if [ "$NEED_DOCKER" = 1 ]; then bad "docker compose plugin missing (apt install docker-compose-plugin)"; exit 1; fi
  echo "   compose plugin missing (not needed for native mode)"
else
  HAVE_DOCKER=1
  docker --version
  ok "engine reachable as $(id -un) · $(docker compose version)"
  echo "cgroup: $(docker info -f '{{.CgroupDriver}} v{{.CgroupVersion}}' 2>/dev/null) · storage: $(docker info -f '{{.Driver}}' 2>/dev/null)"
fi

say "Configuration (runtime/.env)"
if [ ! -f "$RT/.env" ]; then
  cp "$RT/.env.example" "$RT/.env" && chmod 600 "$RT/.env"
  rnd() { head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' | cut -c1-"$1"; }
  sed -i "s/^SMT5_WEB_PASSWORD=.*/SMT5_WEB_PASSWORD=$(rnd 18)/; s/^SMT5_API_TOKEN=.*/SMT5_API_TOKEN=$(rnd 48)/" "$RT/.env"
  ok "created with a random noVNC password and API token (see runtime/.env)"
else
  ok "exists (kept)"
fi
STATUS_PORT="$(sed -n 's/^SMT5_STATUS_PORT=//p' "$RT/.env")"; STATUS_PORT="${STATUS_PORT:-8002}"
DISPLAY_PORT="$(sed -n 's/^SMT5_DISPLAY_PORT=//p' "$RT/.env")"; DISPLAY_PORT="${DISPLAY_PORT:-6081}"

if [ "$CLI" = 1 ]; then
  say "smt5 CLI"
  if command -v cargo >/dev/null; then
    t0=$(date +%s)
    if (cd app && cargo build --release -q -p smt5-cli); then
      ok "built in $(( $(date +%s) - t0 )) s"
      ./app/target/release/smt5 --version
      ./app/target/release/smt5 doctor --runtime-dir "$RT" || true
    else bad "cargo build failed"; fi
  else
    bad "cargo missing (curl https://sh.rustup.rs -sSf | sh), CLI skipped"
  fi
fi

NSH="$RT/macos/native.sh"
if [ "$NATIVE" = 1 ] && [ "$AFTER" = 0 ]; then
  say "Linux native mode (no Docker)"
  missing=""
  for t in Xvfb x11vnc websockify; do command -v "$t" >/dev/null 2>&1 || missing="$missing $t"; done
  [ -f /usr/share/novnc/vnc.html ] || missing="$missing novnc"
  python3 -c 'import venv, ensurepip' 2>/dev/null || missing="$missing python3-venv"
  if [ -n "$missing" ] && [ -z "${DISPLAY:-}" ]; then
    bad "missing for a headless run:$missing"
    echo "   sudo apt install xvfb x11vnc websockify novnc openbox python3-venv"
    exit 1
  fi
  ok "tools present (headless: Xvfb + x11vnc + noVNC)"
  if [ "$HAVE_DOCKER" = 1 ] && docker inspect -f '{{.State.Running}}' siliconmt5 2>/dev/null | grep -q true; then
    echo "   the Docker mode container uses the same ports: stopping it (data kept)"
    dc stop >/dev/null
  fi
  t0=$(date +%s)
  SMT5_ENV_FILE="$RT/.env" bash "$NSH" install && ok "install in $(( $(date +%s) - t0 )) s" || { bad "install failed"; exit 1; }
  t0=$(date +%s)
  SMT5_ENV_FILE="$RT/.env" bash "$NSH" start || { bad "start failed"; exit 1; }
  last=""
  for _ in $(seq 1 60); do
    phase="$(api /status 5 | pyj 'print(d.get("phase",""), "|", d.get("detail",""))')"
    [ "$phase" != "$last" ] && echo "   $(( $(date +%s) - t0 )) s  $phase" && last="$phase"
    case "$phase" in waiting_login*|ready*|error*) break ;; esac
    sleep 5
  done
  case "$last" in
    waiting_login*|ready*) ok "MT5 + bridge up in $(( $(date +%s) - t0 )) s" ;;
    *) bad "did not reach waiting_login: $last"; tail -40 "${XDG_DATA_HOME:-$HOME/.local/share}/siliconmt5/logs/supervisor.log" ;;
  esac
  echo "   display: $(cat "${XDG_DATA_HOME:-$HOME/.local/share}/siliconmt5/state/display" 2>/dev/null)"
  bash "$NSH" status
fi

if [ "$AFTER" = 0 ] && [ "$NATIVE" = 0 ]; then
  say "Image build (no emulation on x86_64; first build ~10–25 min)"
  t0=$(date +%s)
  if dc build; then ok "image built in $(( $(date +%s) - t0 )) s"; else bad "image build failed (see above)"; exit 1; fi
  docker images --format '{{.Repository}}:{{.Tag}}  {{.Size}}' | grep -i siliconmt5 || true

  say "Start"
  t0=$(date +%s)
  dc up -d || { bad "compose up failed"; exit 1; }
  last=""
  for _ in $(seq 1 180); do
    phase="$(api /status 5 | pyj 'print(d.get("phase",""), "|", d.get("detail",""))')"
    [ "$phase" != "$last" ] && echo "   $(( $(date +%s) - t0 )) s  $phase" && last="$phase"
    case "$phase" in waiting_login*|ready*) break ;; error*) break ;; esac
    sleep 10
  done
  case "$last" in
    waiting_login*|ready*) ok "MT5 + bridge up in $(( $(date +%s) - t0 )) s" ;;
    *) bad "did not reach waiting_login within 30 min: $last"; docker logs --tail 60 siliconmt5 ;;
  esac
fi

say "Status API"
api /health 5 | pyj 'print("   health:", d)'
api /status 10 | pyj 'b=d["bridge"]; m=d["mt5"]; print("   phase:", d["phase"], "| mt5 installed:", m["installed"], "running:", m["running"], "maxbars:", m["maxbars"], "| bridge listening:", b["listening"], "| api level:", d.get("api",{}).get("level"), "| cpu:", d["host"]["cpu_translation"], "| disk free MB:", d["disk"]["free_mb"])'
api /status 10 | pyj 'a=(d["bridge"].get("probe") or {}).get("account"); print("   account:", "none (log in via noVNC)" if not a else " ".join(str(a.get(k)) for k in ("server", "login_masked", "trade_mode")))'

if [ "$AFTER" = 1 ]; then
  say "Data test (EURUSD M5, same path as the Python client)"
  api "/datatest?symbol=EURUSD&tf=M5&count=300&ticks=5" 200 | pyj 'print("   verdict:", d["verdict"], "|", d.get("message","")); [print("   ", "ok " if c["ok"] else "BAD", c["id"], c.get("ms"), "ms", c.get("detail","")[:90]) for c in d.get("checks",[])]'
  say "Supertrend (EURUSD H1 3000, EURUSD D1 3000)"
  for tf in H1 D1; do
    t0=$(date +%s%N)
    api "/strategy/supertrend?symbol=EURUSD&tf=$tf&period=10&multiplier=3&bars=3000" 120 | pyj 's=d.get("stats"); print("   '"$tf"': error:", d["error"]) if "error" in d else print("   '"$tf"':", d["count"], "closed bars of", d.get("requested"), "| trend", d["trend"], "| signals", len(d["signals"]), "| trades", s["trades"], "| net", round(s["net"],5))'
    echo "      $(( ($(date +%s%N) - t0) / 1000000 )) ms"
  done
  say "Account history (30 days, numbers only)"
  api "/account/history?range=30d" 80 | pyj 'print("   error:", d["error"]) if "error" in d else print("   deals", d.get("deals"), "| closed trades", d.get("trades"), "| trading P/L", d.get("trading_pl"), "| deposits", d.get("deposits"), "| floating", d.get("floating"))'
  say "Speed test"
  api "/bench?symbol=EURUSD&bars=50000" 330 | pyj 'print("   ", {k: d.get(k) for k in ("ok","error","boot_s","cpu_s","rates_rows","rates_s","transfer_rows","transfer_s","tick_ms","wine_version","cpu_translation")})'
  say "Resources"
  if [ "$HAVE_DOCKER" = 1 ] && docker inspect siliconmt5 >/dev/null 2>&1; then
    docker stats --no-stream --format '   {{.Name}}  CPU {{.CPUPerc}}  RAM {{.MemUsage}}' siliconmt5 2>/dev/null
  else
    # native mode: only the processes of our own systemd unit (or agent.pid's
    # session), never other jobs on the machine that happen to be python
    cg="$(systemctl --user show -p ControlGroup --value siliconmt5-runtime 2>/dev/null)"
    pids=""
    if [ -n "$cg" ] && [ -r "/sys/fs/cgroup$cg/cgroup.procs" ]; then
      pids="$(paste -sd, "/sys/fs/cgroup$cg/cgroup.procs")"
    else
      ap="$(cat "${XDG_DATA_HOME:-$HOME/.local/share}/siliconmt5/state/agent.pid" 2>/dev/null)"
      [ -n "$ap" ] && pids="$(ps -eo pid=,sid= | awk -v s="$ap" '$2 == s {print $1}' | paste -sd,)"
    fi
    if [ -n "$pids" ]; then
      ps -o pcpu=,rss=,args= -p "$pids" | awk '{a=$0; sub(/^ *[^ ]+ +[^ ]+ +/, "", a); if (match(a, /[^\/\\]*\.exe/)) n=substr(a, RSTART, RLENGTH); else {n=a; sub(/ .*/, "", n); sub(/.*\//, "", n)}; printf "   %-22s CPU %5s%%  RAM %6.0f MB\n", substr(n,1,22), $1, $2/1024; c+=$1; r+=$2}
        END {printf "   %-22s CPU %5.1f%%  RAM %6.0f MB\n", "TOTAL", c, r/1024}'
      [ -r "/sys/fs/cgroup$cg/memory.current" ] && echo "   (cgroup memory incl. cache: $(( $(cat "/sys/fs/cgroup$cg/memory.current") / 1048576 )) MB)"
    else
      echo "   (runtime processes not found)"
    fi
  fi
fi

say "Result: $FAILS failure(s)"
if [ "$AFTER" = 0 ]; then
  SHOWN="$(cat "${XDG_DATA_HOME:-$HOME/.local/share}/siliconmt5/state/display" 2>/dev/null)"
  if [ "$NATIVE" = 1 ] && [ "$SHOWN" = window ]; then
    cat <<EOF
Next: the MT5 window is open on this machine's own screen (DISPLAY=${DISPLAY:-}).
  MT5: File › Login to Trade Account. When the bottom-right shows the connection, run:
    bash packaging/linux/test-linux.sh --after-login
  (to use it from another computer instead: bash runtime/macos/native.sh stop, then
   SMT5_NATIVE_DISPLAY=novnc bash packaging/linux/test-linux.sh --native)
EOF
  else
    cat <<EOF
Next: log in to your broker in MT5 through the noVNC desktop.
  From your Mac:  ssh -N -L $DISPLAY_PORT:127.0.0.1:$DISPLAY_PORT $(id -un)@$(hostname -I 2>/dev/null | awk '{print $1}')
  Then open http://127.0.0.1:$DISPLAY_PORT/vnc.html?autoconnect=1&resize=scale (password: SMT5_WEB_PASSWORD in runtime/.env on this machine)
  MT5: File › Login to Trade Account. When the bottom-right shows the connection, run:
    bash packaging/linux/test-linux.sh --after-login
EOF
  fi
  cat <<EOF
Stop it later with:
  native mode:  bash runtime/macos/native.sh stop
  Docker mode:  docker compose -f runtime/compose.yaml --env-file runtime/.env down   (data volumes are kept)
EOF
fi
echo "Log: $LOG"
