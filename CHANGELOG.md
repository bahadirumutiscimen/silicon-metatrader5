# Changelog

## 1.3.0

The desktop app release. Details and numbers: [release notes](docs/releases/v1.3.0.md).

- Desktop app (macOS, Linux): setup wizard, overview, MT5 screen, logs, data and speed tests, data management, accounts, settings, menu bar panel.
- Mac native mode: MT5 under Rosetta 2 without a VM; up to 30× faster than 1.2 on an Apple M4.
- Linux native mode (Wine 11, with or without a display) next to the Docker mode.
- Servers: Linux (native or Docker) and Windows VPS managed over SSH, with an encrypted tunnel, clipboard sync and a bot connection guide.
- `smt5` command line with `--json` on every command.
- Python client 1.3.0: broker server-time helpers (`server_now`, `server_time_offset`, `format_offset`, `bar_time`), Python 3.14.
- License: PolyForm Noncommercial 1.0.0.
- Removed: `docker/` and `docker_kasm/` (replaced by `runtime/`).

## 1.2.3

- Python client: packaged bridge server layout fixed.
