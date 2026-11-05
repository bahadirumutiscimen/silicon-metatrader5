# SiliconMetaTrader5 runtime (1.3.0)

MetaTrader 5 under Wine, the `siliconmetatrader5` Python bridge and a small
status/data API — in two forms:

* **Docker image** (`Dockerfile`, `compose.yaml`): Wine + a noVNC web desktop.
  Used by the macOS **Safe** mode (inside a Colima x86_64 QEMU VM) and on
  **Linux x86_64**.
* **Mac native** (`macos/native.sh`): the same pieces directly in macOS Wine 11
  under Rosetta 2, supervised by a launchd agent. No VM, no Docker.

Day to day you use the desktop app or the `smt5` CLI (`../app`); this page is
the reference underneath.

| Port (host, `127.0.0.1` only) | What |
| --- | --- |
| `6081` | noVNC web desktop with MT5 (Docker only; Mac native shows a macOS window) |
| `8001` | Python bridge — `MetaTrader5(host="127.0.0.1", port=8001)` |
| `8002` | Status / data API (see below) |

> **Security.** The bridge executes arbitrary Python inside the runtime and can
> place orders on your account. All ports are bound to `127.0.0.1` only. For
> remote use, tunnel: `ssh -L 8001:127.0.0.1:8001 -L 6081:127.0.0.1:6081 server`.

## Quick start

```bash
smt5 init        # runtime/.env with a noVNC password and an API token
smt5 doctor      # what is missing, with the exact fix
smt5 build       # Docker image (or Mac native install); `smt5 pull` for the CI image
smt5 up
smt5 watch       # phase every 15 s until "ready"
```

Log in to your broker in MT5 (`File › Open an Account`), then check that data
really flows: `smt5 test EURUSD`, or `python examples/fetch_data.py` from the repo root.

Plain Docker works too: `cp .env.example .env`, fill in `SMT5_WEB_PASSWORD` and
`SMT5_API_TOKEN` (`openssl rand -hex 24`), then `docker compose up -d` and
open <http://127.0.0.1:6081>.

## Hosts

### macOS · Mac native (recommended)

`macos/native.sh` runs MT5 and the bridge in macOS Wine 11 (x86_64, under the
Mac's own Rosetta 2 — the way MetaQuotes' MetaTrader 5 for Mac works). Needs
Rosetta 2 and a python3 ≥ 3.9 (`xcode-select --install`); no Homebrew packages.

```bash
smt5 mode native     # SMT5_PROVIDER=mac-native in .env
smt5 build           # Wine 11.21 (Gcenx build, SHA-256 pinned), Windows Python, MT5, status API venv
smt5 up              # launchd agent com.siliconmt5.runtime: MT5 window + bridge :8001 + status :8002
smt5 down            # MT5 closes gracefully (~2 s), then the agent stops
```

Everything lives in `~/Library/Application Support/SiliconMT5`: `wine/`,
`prefix/` (Wine prefix with MT5), `venv/` (status API), `state/`, `logs/` and
`runtime/`, a copy of the scripts the agent runs so macOS never asks for
Documents access in the background. The agent is bootstrapped from `state/`,
not `~/Library/LaunchAgents`, so it does not start at login. The first `build`
copies MT5 with the broker login and history from the Safe-mode container when it
exists (`smt5 mt5-import` repeats that later, keeping a dated backup); otherwise
the MetaQuotes installer runs. `supervisor.log` records who requested a stop
and which signal the supervisor received.

Closing the MT5 window (or `File › Exit`) stops the whole runtime, like
`smt5 down`; set `SMT5_MT5_ON_CLOSE=restart` in `.env` to reopen it instead. A
crash is restarted, but not more than three times within 90 s of starting.

`native.sh` can also be used directly: `install`, `start`, `stop`, `restart`,
`status [--json]`, `run` (foreground), `import`, `reset` (Wine prefix),
`reset-all`, `uninstall`.

### macOS · Safe (QEMU)

The image is `linux/amd64` because MT5 is an x86_64 Windows program. On Apple
Silicon it must run in a **full x86_64 VM**: Rosetta inside a Linux VM (Docker
Desktop, OrbStack, Colima `vz`) and qemu-user crash Wine (`terminal64.exe`
segfaults, `x86_avx_state_ptr->xsave_header` assertions in `wineboot`). The
build and the supervisor detect this and stop with a clear error.

```bash
brew install colima docker docker-compose qemu lima-additional-guestagents
smt5 mode safe
smt5 vm start    # = colima start --profile x86 --arch x86_64 --vm-type=qemu --cpu 4 --memory 8 --disk 60
smt5 build       # 30–90 min under QEMU; keeps the Mac awake (a sleep can corrupt the export)
```

Prefer the CI image when possible: every push to `main` publishes
`ghcr.io/bahadirumutiscimen/siliconmt5:edge`; set `SMT5_VERSION=edge` in `.env`
and run `smt5 pull`. If you build locally, turn off the containerd image store
in the x86 profile (`colima start --profile x86 --edit`, set
`docker: {features: {containerd-snapshotter: false}}`).

**Next to other Docker projects** (e.g. an arm64 project on the default Colima
profile): `smt5` and the app pin `DOCKER_CONTEXT=colima-x86` for their own
commands only, so `docker context use colima` of the other projects stays as it
is. Containers of other projects should not run inside the x86 VM — everything
there is emulated and competes with MT5 (`smt5 doctor` warns). Both VMs together
need RAM: 10 GiB (default) + 8 GiB (x86) does not fit a 16 GB Mac; lower one of
them with `colima start --profile <name> --edit`.

`docker compose` is a CLI plugin: if it disappears (e.g. after removing Docker
Desktop or OrbStack), link Homebrew's:
`mkdir -p ~/.docker/cli-plugins && ln -sfn "$(brew --prefix)/opt/docker-compose/bin/docker-compose" ~/.docker/cli-plugins/docker-compose`.

On an Intel Mac the VM needs no emulation: `colima start --cpu 4 --memory 8`.

### Linux x86_64 · Linux native (no Docker)

The same `macos/native.sh` runs on Linux x86_64 with a pinned Wine 11 build
(Kron4ek 11.21 WoW64, SHA-256 pinned; 64-bit host libraries only). Files live
in `${XDG_DATA_HOME:-~/.local/share}/siliconmt5`; the supervisor runs as a
transient `systemd --user` unit (`siliconmt5-runtime`, not started at login) or,
without a user systemd, as a detached process.

```bash
smt5 mode native && smt5 build && smt5 up
```

On a desktop MT5 is a normal window. Headless (no `DISPLAY`) the supervisor
starts its own Xvfb + x11vnc (localhost) + noVNC on `127.0.0.1:SMT5_DISPLAY_PORT`
(`sudo apt install xvfb x11vnc websockify novnc openbox python3-venv`);
`SMT5_NATIVE_DISPLAY=window|novnc` forces either. Wine runs natively on x86_64,
so it is as fast as the Docker image; Docker stays the choice for servers that
want the pinned, isolated image.

### Linux x86_64 · Docker

Docker Engine, nothing else; no emulation, the fastest Docker option.

### Server mode (SSH, desktop app)

The setup wizard's **Sunucu (SSH)** card installs the runtime on a Linux
x86_64 VPS and the **Sunucular** page manages any number of them:

* Login is with an SSH key only (`ssh -o BatchMode=yes`); the app never asks
  for or stores a password. Without a key it creates
  `~/.ssh/siliconmt5_ed25519` and opens Terminal with `ssh-copy-id`.
* The server's host key is shown and confirmed once; the app keeps its own
  `known_hosts` (`<app config>/ssh/known_hosts`) and refuses a changed key.
* A precheck (`runtime/server/preflight.sh`) lists what is missing; packages
  and `loginctl enable-linger` (needed so Linux native survives SSH logout)
  are installed with one `sudo` command the user runs in Terminal.
* On the server everything lives in `~/siliconmetatrader5/runtime` (its own
  `.env`, password and token made there) plus the usual
  `~/.local/share/siliconmt5` (Linux native) or Docker volumes.
* The app keeps one `ssh -N -L` tunnel per server: local block
  `18000 + 100·n` → bridge `+1`, status API `+2`, noVNC `+81`. Code on your
  computer reaches a server's bridge with `MetaTrader5(port=18001)`.

### Windows server (`runtime/windows`)

Windows x64 VPS (10 1809+ / Server 2019+), driven over SSH by the app; MT5
runs natively, no Wine. The setup wizard installs it; the files are:

* `smt5.ps1`: `install`, `start`, `stop`, `restart`, `reset`, `uninstall`,
  `autologon` (`off` reverts it), `autologon-status`, `session`, `reboot`,
  `stats` (one `SMT5STATS win <cpu%> <used> <total>` line); helpers in `lib/`.
* `supervisor.py` (task `SiliconMT5-Supervisor`, at the user's logon): keeps
  MT5, the bridge and the status API running; writes `mt5cfg.ini` from the
  template and `.env` before each MT5 start; logs in `logs\` are rotated at
  `SMT5_LOG_MAX_MB`.
* `display.py` (task `SiliconMT5-Display`, SYSTEM, at startup): websockify
  (noVNC) in front of the TightVNC service, both on 127.0.0.1, logging to
  `logs\display.log`. It does not need a user session: with nobody logged on
  the screen shows the Windows logon screen. `stop` leaves it running,
  uninstall removes it.
* `preflight.ps1`: precheck of what is missing.
* `versions.json`: pinned Python, TightVNC, noVNC and MT5 downloads. Each is
  verified by `sha256`, or by the Authenticode signer when the pin has no
  `sha256` (TightVNC: GlavSoft, MT5: MetaQuotes).

On the server: `%LOCALAPPDATA%\siliconmetatrader5` (runtime, state, logs) and
`C:\Program Files\MetaTrader 5` with a `.siliconmt5` marker; a foreign MT5
without the marker is left alone. `state\installed` is written only when
`smt5.ps1 install` finished; until then the app shows the server as not
installed. The install log is `logs\installer.log`.

Settings on Windows: `SMT5_MAXBARS`, `SMT5_ALGO_TRADING`, `SMT5_DLL_IMPORT`
(`mt5cfg.ini`), `SMT5_LOG_MAX_MB`, `SMT5_RESOLUTION` and the ports apply;
"Save and apply" runs stop, install (idempotent: TightVNC port, display
task) and start. `SMT5_DPI` is Wine only.

### Remote Linux (Docker context)

`docker context create box --docker host=ssh://user@box`, set
`SMT5_DOCKER_CONTEXT=box` and tunnel the ports as shown above.

Linux ARM64 is not supported.

## Configuration (`.env`)

In a repository checkout the settings file is `runtime/.env`. An installed app
(Homebrew, .deb, AppImage) keeps it in `~/Library/Application Support/SiliconMT5/.env`
(macOS) or `~/.config/siliconmt5/.env` (Linux); `SMT5_ENV_FILE` overrides both.

| Variable | Default | Notes |
| --- | --- | --- |
| `SMT5_PROVIDER` | empty | macOS: empty = Safe, `mac-native` = Mac native (`smt5 mode`) |
| `SMT5_DOCKER_CONTEXT` | `colima-x86` (macOS) | Docker context pinned for SiliconMetaTrader5's commands |
| `SMT5_VERSION` | empty | Image tag; empty = the version of smt5 / the app, `edge` = latest `main` build |
| `SMT5_WEB_PASSWORD` | empty | noVNC password |
| `SMT5_API_TOKEN` | empty | Enables the mutating API endpoints (purge, restart, log clearing) |
| `SMT5_MAXBARS` | `100000` | Bars per chart. 1,000,000 froze history sync under QEMU |
| `SMT5_ALGO_TRADING` | `1` | MT5's Algo Trading button and live trading by EAs |
| `SMT5_DLL_IMPORT` | `0` | DLL imports for EAs |
| `SMT5_LOG_MAX_MB` | `10` | Each log is rotated above this size (one old copy kept) |
| `SMT5_RESOLUTION` | `1366x768` | noVNC screen size |
| `SMT5_DPI` | `96` | MT5 text and button size in noVNC (96 = 100 %, 120 = 125 %) |
| `TZ` | `Europe/Istanbul` | Container clock only, not broker time |
| `SMT5_BIND` | `127.0.0.1` | Host bind address for the Docker ports |
| `SMT5_DISPLAY_PORT` / `SMT5_BRIDGE_PORT` / `SMT5_STATUS_PORT` | `6081` / `8001` / `8002` | Host ports |

`SMT5_MAXBARS`, `SMT5_ALGO_TRADING` and `SMT5_DLL_IMPORT` are written into MT5's
startup config (`mt5cfg.ini`) only when they change, so changes made inside MT5
survive restarts.

## Status & data API (`:8002`)

Read-only (no token):

```bash
curl -s 127.0.0.1:8002/status            # phase: preparing_prefix → installing_mt5 → starting_terminal
                                         #        → starting_bridge → waiting_login → ready
curl -s '127.0.0.1:8002/logs?source=mt5&lines=100'   # supervisor|mt5|bridge|installer|display|status
curl -s 127.0.0.1:8002/data              # history (.hcc per year), ticks (.tkc per month), cache, Tester sizes
curl -s '127.0.0.1:8002/datatest?symbol=EURUSD&tf=M5'   # bars + ticks through the bridge
curl -s 127.0.0.1:8002/symbols           # Market Watch
curl -s '127.0.0.1:8002/bench?symbol=EURUSD'            # speed test
```

Mutating (`Authorization: Bearer $SMT5_API_TOKEN`):

```bash
# Plan first (dry run is the default) …
curl -s -X POST 127.0.0.1:8002/data/purge -H "Authorization: Bearer $SMT5_API_TOKEN" \
  -d '{"items":[{"kind":"history","symbol":"*","before_year":2023},{"kind":"ticks","symbol":"*","before_month":202608}]}'
# … then delete. MT5 is stopped gracefully, files are removed, MT5 restarts (~30 s).
curl -s -X POST 127.0.0.1:8002/data/purge -H "Authorization: Bearer $SMT5_API_TOKEN" \
  -d '{"dry_run":false,"items":[{"kind":"cache","symbol":"*"}]}'

curl -s -X POST 127.0.0.1:8002/runtime/restart-mt5    -H "Authorization: Bearer $SMT5_API_TOKEN"
curl -s -X POST 127.0.0.1:8002/runtime/restart-bridge -H "Authorization: Bearer $SMT5_API_TOKEN"
curl -s -X POST 127.0.0.1:8002/logs/clear             -H "Authorization: Bearer $SMT5_API_TOKEN"
```

Purge items: `kind` = `history` (`years` | `before_year` | `all`), `ticks`
(`months` | `before_month` | `all`), `cache`, `tester`; optional `server` and
`symbol` (exact name or `*`). History files are per year and ticks per month,
so that is the finest granularity. Deleted ranges are downloaded again from
the broker when needed.

## Layout

```
runtime/
├── Dockerfile               # one image: Wine + noVNC
├── compose.yaml, .env.example
├── config/                  # mt5cfg.ini, pinned requirements, VC++ DLL overrides
├── scripts/
│   ├── install-system.sh    # build: apt, WineHQ, noVNC, xclip, host venv
│   ├── cpu-translation.sh   # build + run: refuse Rosetta / qemu-user
│   ├── build-runtime.sh     # build: bake Wine prefix (Python, VC++ runtime, bridge)
│   ├── supervisor.sh        # run: prefix → install → MT5 → bridge → watch
│   ├── prepare-prefix.sh, install-mt5.sh, launch-mt5.sh, launch-bridge.sh
│   ├── entrypoint-novnc.sh  # Xvfb + x11vnc + noVNC + status API + supervisor
│   └── common.sh            # run-time paths and helpers
├── status/                  # :8002 API + data test / bench (+ tests; also runs on macOS)
└── macos/native.sh          # Mac native mode: Wine 11 install, launchd supervisor
```

## Upgrading from 1.2.x (`docker/`, `docker_kasm/`)

The old folders and the KasmVNC variant are gone. The compose project uses new
volume names, so the old ones are left untouched: a fresh start installs MT5
again and you log in to your broker once more. Remove the old volumes when you
are happy:
`docker volume rm docker_wineprefix_persistence siliconmetatrader5-kasm_siliconmetatrader5_kasm_config`.
