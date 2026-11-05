# SiliconMetaTrader5 app (Rust)

Cargo workspace shared by the `smt5` command line (Phase 1) and the Tauri v2
desktop app (Phase 2).

| Crate | What |
| --- | --- |
| `crates/smt5-core` | Providers (Mac native, Colima x86_64 VM, Docker Engine, SSH), runtime modes, the Mac native paths and facts (`native`), docker/compose calls pinned to the SiliconMetaTrader5 context, typed client for the status API (`:8002`), doctor checks |
| `crates/smt5-cli` | The `smt5` binary |
| `crates/smt5-desktop` | The desktop app (`siliconmt5`): Tauri v2 shell + static HTML/JS UI in `ui/` (no Node build step) |

## Build

```bash
# once: Rust toolchain (Homebrew's rustup is keg-only and has no rustup-init)
brew install rustup
echo 'export PATH="/opt/homebrew/opt/rustup/bin:$PATH"' >> ~/.zshrc
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
rustup default stable
# (alternative: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh)

cd app
cargo build --release                      # → app/target/release/smt5 (CLI only; add -p smt5-desktop for the app)
cargo install --path crates/smt5-cli       # optional: puts `smt5` on your PATH
```

`cargo build` builds the CLI only. The desktop app:

```bash
cd app
cargo run -p smt5-desktop --release              # build and start the window
# Linux needs WebKitGTK first:
#   sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev

# optional: a real SiliconMetaTrader5.app / .deb / AppImage (bundles runtime/ as a resource)
cargo install tauri-cli --version '^2' --locked
cd crates/smt5-desktop && cargo tauri build --bundles app   # → app/target/release/bundle/macos/SiliconMetaTrader5.app
```

## Packaging and releases

* **Homebrew** (macOS): `packaging/homebrew/siliconmt5.rb` builds from source on
  the user's Mac (no quarantine, ad-hoc signed bundle), installs `smt5`,
  `SiliconMetaTrader5.app` and the runtime under `share/siliconmt5/runtime`. Formulae
  cannot write to /Applications and Spotlight ignores links, so
  `smt5 app-install` copies the bundle there (`smt5_core::macapp`); after
  `brew upgrade` the app notices the older copy and refreshes it. Try it
  before a release with `bash packaging/homebrew/test-local.sh` (local tap
  `smt5/local`).
* **Linux**: `.github/workflows/release.yml` builds the AppImage and the .deb
  (with `/usr/bin/smt5`) on a `vX.Y.Z` tag, renders the formula with the
  source tarball's sha256, commits it as `Formula/siliconmt5.rb` (this repository
  is the Homebrew tap) and publishes the GitHub release with
  `docs/releases/vX.Y.Z.md` as its notes.
* **Settings of installed apps**: a packaged runtime folder carries a
  `.packaged` marker; its settings live in `~/Library/Application
  Support/SiliconMT5/.env` (macOS) or `~/.config/siliconmt5/.env` (Linux) and
  are passed to compose with `--env-file`. A checkout keeps `runtime/.env`.
  `SMT5_ENV_FILE` overrides both.
* **Updates**: `brew upgrade siliconmt5` / a new .deb or AppImage. The app checks
  GitHub's latest release at most once a day and shows how to update.

The CLI finds `runtime/` from the current directory (or pass
`--runtime-dir`), then next to its own binary (package layouts). It never runs `docker context use`: every docker command is
pinned with `DOCKER_CONTEXT` for that process only (`colima-x86` on macOS).

## Commands

```text
smt5 doctor                  prerequisites + health, with the command that fixes each problem
smt5 init                    create runtime/.env, generate desktop password and API token
smt5 mode [safe|native]      show or switch the macOS runtime mode (stops the running one first)
smt5 vm start|stop|status    Colima x86_64 VM of the Safe mode (macOS)
smt5 build | pull | up | down [-v --yes] | restart [container|mt5|bridge]
                             (Mac native: build = install, up/down = launchd agent, -v = delete the Wine prefix)
smt5 status | watch          phase, elapsed time, account / probe reason
smt5 logs [supervisor|mt5|bridge|installer|display|status] [-n 80]
smt5 data                    MT5 history / tick / tester sizes per server and symbol
smt5 purge --kind history --server Default --all [--yes]      dry run unless --yes
smt5 purge --kind history --symbol EURUSD --before-year 2024 --yes
smt5 purge --kind ticks --before-month 202608 --yes
smt5 test [EURUSD] [--tf M5] data test through the bridge, on the broker clock
smt5 symbols                 Market Watch symbols
smt5 bench [EURUSD] [--bars 50000]   speed test: boot, Windows-Python CPU, copy_rates, transfer, latency
smt5 mt5-import              Mac native: copy MT5 (login + history) from the Safe-mode container again
smt5 shell | compose …       Docker modes only
```

`--json` gives machine-readable output for `doctor`, `status`, `data`,
`purge`, `test` and `vm status` (the desktop app uses the same core directly).

## Desktop app

Screens: Overview (account, session equity curve, runtime health, resources),
MT5 screen (noVNC embedded with the password passed in the URL fragment, or
"bring the MT5 window to front" in the Mac native mode), Logs, Data test, Runtime settings (`.env`), Data
management (per-symbol bar years, quick clean, MaxBars) and the setup wizard
(mode + system check → x86_64 VM or Wine/MT5 install → configuration → runtime → broker login →
data test). Long jobs (VM start, image build, compose up/down) stream their
output into a drawer; on macOS a menu-bar icon shows the phase and closing the
window keeps the app there.

Runtime states come from one place (`backend::lifecycle`): VM/Docker off,
not configured, no image, never started, stopped (data kept), starting, then
the supervisor phases. **Start** brings the VM up first when needed; **Stop**
is `compose stop` (the container stays). Settings → *Reset and uninstall* has
three levels, each listing what is deleted and kept and asking for a typed
confirmation: MT5 data only (`compose down -v`), delete our Colima profile and
redo the wizard, or remove everything SiliconMetaTrader5 created (optionally its
Homebrew packages, only when no other Colima profile exists). Other projects'
Colima profiles, Docker itself and the repository are never touched.

**Runtime modes** on macOS (`smt5 mode`, Settings → Runtime mode, first wizard
step): `safe` = Colima x86_64 QEMU VM + WineHQ container (isolated, slow on
ARM); `native` = MT5 and the bridge directly in macOS Wine 11 under Rosetta 2
(`runtime/macos/native.sh`, a launchd agent, everything in
`~/Library/Application Support/SiliconMT5`; no VM, no Docker; the MT5 window
is a macOS window). The first native install copies MT5 with its broker login
from the safe container when it exists. Both modes keep their own data and use
the same 127.0.0.1 ports, so only one runs at a time. `smt5 bench` / Data test
→ Speed test measures boot, Windows-Python CPU, copy_rates_from_pos, bridge
transfer and latency, and keeps one result per mode for comparison.

With noVNC the MT5 screen keeps the host clipboard and MT5's clipboard in sync
(`xclip` on the container's X display) while the screen is open: copy on the
Mac, Ctrl+V in MT5; Ctrl+C in MT5, ⌘V on the Mac.

The app looks for `runtime/` in its saved setting, then `$SMT5_RUNTIME_DIR`,
the current directory and the folders above the binary; otherwise it asks for
the repository folder once. GUI apps on macOS start without Homebrew in `PATH`,
so the app adds `/opt/homebrew/bin` and `/usr/local/bin` itself.

UI development without Tauri: serve `crates/smt5-desktop/ui/` with any static
server and open `index.html?mock=ready` (also `login`, `booting`, `error`,
`none`; add `&native=1` for the Mac native mode) — `ui/js/mock.js` answers
every command with realistic data.

## Develop

```bash
cargo test --workspace            # 44 unit tests (mocked runner, mocked HTTP, temp .env files)
cargo clippy --workspace --all-targets
```

`runtime/smt5` (bash) is the earlier prototype of this CLI; it stays until the
Phase 3 cleanup.
