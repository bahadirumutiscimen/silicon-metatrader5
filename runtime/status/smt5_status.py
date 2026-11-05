#!/usr/bin/env python3
"""SiliconMetaTrader5 status & data API (runs inside the runtime container on :8002).

Read-only endpoints (no token):
    GET  /health                      liveness for the container healthcheck
    GET  /status                      runtime phase, processes, bridge probe
    GET  /logs?source=mt5&lines=200   tail of a runtime log
    GET  /data                        MT5 history / tick / tester storage inventory
    GET  /datatest?symbol=EURUSD&tf=M5&count=100&ticks=5
                                      setup-wizard data test through the bridge

Mutating endpoints (``Authorization: Bearer $SMT5_API_TOKEN``):
    POST /data/purge                  plan (dry run, default) or delete history files
    POST /runtime/restart-mt5         graceful terminal restart
    POST /runtime/restart-bridge      bridge restart
    POST /clipboard/get               MT5's clipboard text (Server mode clipboard sync)
    POST /clipboard/set               {"text": ...} → MT5's clipboard

Deleting files while MT5 runs can corrupt its history cache, so a real purge
asks the supervisor to stop the terminal first (maintenance.request →
maintenance.ready), deletes, and lets the supervisor start it again.

Only the Python standard library is required; the bridge probe additionally
uses the ``siliconmetatrader5`` client when it is importable.
"""
from __future__ import annotations

import hmac
import json
import os
import re
import socket
import sys
import threading
import time
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional
from urllib.parse import parse_qs, urlparse

VERSION = "1.3.0"
# Bumped when endpoints are added: the app compares it to tell a runtime that
# still runs older scripts (restart needed) from a current one.
API_LEVEL = 3

# --------------------------------------------------------------------------
# configuration (environment, read once at import)
# --------------------------------------------------------------------------
# Windows server mode runs this service under the server's own Windows Python.
# SMT5_PLATFORM=windows also lets the Windows branch be exercised on Linux.
IS_WINDOWS = os.environ.get("SMT5_PLATFORM") == "windows" or sys.platform == "win32"
WINEPREFIX = Path(os.environ.get("WINEPREFIX", "/config/.wine"))
STATE_DIR = Path(os.environ.get("SMT5_STATE_DIR", "/config/.siliconmt5"))
LOG_DIR = Path(os.environ.get("SMT5_LOG_DIR", "/siliconmt5/logs"))
MT5_DIR = Path(os.environ.get("SMT5_MT5_DIR", str(WINEPREFIX / "drive_c" / "Program Files" / "MetaTrader 5")))
BRIDGE_PORT = int(os.environ.get("SMT5_BRIDGE_PORT_INTERNAL", "8001"))
LISTEN_HOST = os.environ.get("SMT5_STATUS_LISTEN", "0.0.0.0")
LISTEN_PORT = int(os.environ.get("SMT5_STATUS_PORT_INTERNAL", "8002"))
API_TOKEN = os.environ.get("SMT5_API_TOKEN", "")
DISPLAY = os.environ.get("SMT5_DISPLAY", "novnc")
PROBE_INTERVAL = float(os.environ.get("SMT5_PROBE_INTERVAL", "20"))
PROBE_TIMEOUT = float(os.environ.get("SMT5_PROBE_TIMEOUT", "60"))  # first initialize() under QEMU is slow
MAINTENANCE_TIMEOUT = float(os.environ.get("SMT5_MAINTENANCE_TIMEOUT", "90"))
MIN_FREE_MB = int(os.environ.get("SMT5_MIN_FREE_MB", "512"))
ALLOWED_HOSTS = {"127.0.0.1", "localhost", "::1", "[::1]"} | {
    h.strip().lower() for h in os.environ.get("SMT5_ALLOWED_HOSTS", "").split(",") if h.strip()
}

LOG_SOURCES = {
    "supervisor": "supervisor.log",
    "mt5": "mt5.log",
    "bridge": "bridge.log",
    "installer": "installer.log",
    "display": "display.log",
    "status": "status.log",
}
SUPERVISOR_ONLY_PHASES = {"preparing_prefix", "installing_mt5", "error", "maintenance", "stopping"}
TRADE_MODES = {0: "demo", 1: "contest", 2: "real"}

# Server and symbol directory names: no path separators, no leading dot.
NAME_RE = re.compile(r"^[A-Za-z0-9#][A-Za-z0-9 ._#+&()!$-]{0,63}$")
HCC_RE = re.compile(r"^(\d{4})\.hcc$", re.IGNORECASE)
TKC_RE = re.compile(r"^(\d{6})\.tkc$", re.IGNORECASE)
MAX_BODY = 64 * 1024
MAX_ITEMS = 500


class ApiError(Exception):
    def __init__(self, status: HTTPStatus, message: str):
        super().__init__(message)
        self.status = status
        self.message = message


# --------------------------------------------------------------------------
# small helpers
# --------------------------------------------------------------------------
def port_open(port: int, host: str = "127.0.0.1", timeout: float = 1.0) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except OSError:
        return False


def process_running(needle: str) -> bool:
    """True if any process command line contains ``needle`` (case-insensitive).
    Reads /proc on Linux; uses ``ps`` where there is none (macOS native mode) and
    PowerShell on Windows (``wmic`` is gone from current Windows builds)."""
    needle_b = needle.lower().encode()
    if IS_WINDOWS:
        import subprocess

        try:
            out = subprocess.run(
                ["powershell", "-NoProfile", "-Command",
                 "Get-CimInstance Win32_Process | % { $_.CommandLine }"],
                capture_output=True, timeout=5, stdin=subprocess.DEVNULL,
                creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000)).stdout
        except (OSError, subprocess.SubprocessError):
            return False
        return any(needle_b in line.lower() for line in out.splitlines())
    if not os.path.isdir("/proc/self"):
        import subprocess

        try:
            out = subprocess.run(["ps", "-axww", "-o", "command="], capture_output=True, timeout=5).stdout
        except (OSError, subprocess.SubprocessError):
            return False
        return any(needle_b in line.lower() for line in out.splitlines())
    for entry in os.scandir("/proc"):
        if not entry.name.isdigit():
            continue
        try:
            with open(f"/proc/{entry.name}/cmdline", "rb") as fh:
                if needle_b in fh.read().lower():
                    return True
        except OSError:
            continue
    return False


_KERNEL32: Any = None


def _kernel32() -> Any:
    """kernel32 with explicit prototypes (default restype c_int would truncate
    HANDLE / 64-bit values). Cached."""
    global _KERNEL32
    if _KERNEL32 is None:
        import ctypes
        from ctypes import wintypes

        k32 = ctypes.WinDLL("kernel32", use_last_error=True)  # type: ignore[attr-defined]
        k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k32.OpenProcess.restype = wintypes.HANDLE
        k32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        k32.GetExitCodeProcess.restype = wintypes.BOOL
        k32.CloseHandle.argtypes = [wintypes.HANDLE]
        k32.CloseHandle.restype = wintypes.BOOL
        k32.GetTickCount64.argtypes = []
        k32.GetTickCount64.restype = ctypes.c_ulonglong
        _KERNEL32 = k32
    return _KERNEL32


def _win_pid_alive(pid: int) -> Optional[bool]:
    """Windows liveness without os.kill (there ``os.kill(pid, 0)`` terminates the
    process): OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION) + exit code 259.
    None when the ctypes layer itself fails (caller falls back to the process list)."""
    try:
        import ctypes
        from ctypes import wintypes

        k32 = _kernel32()
        handle = k32.OpenProcess(0x1000, 0, pid)
        if not handle:
            return False
        try:
            code = wintypes.DWORD(0)
            if not k32.GetExitCodeProcess(handle, ctypes.byref(code)):
                return False
            return code.value == 259  # STILL_ACTIVE
        finally:
            k32.CloseHandle(handle)
    except (OSError, AttributeError, ctypes.ArgumentError, ValueError, TypeError):
        return None


def pid_alive(path: Path) -> Optional[bool]:
    """State of the process in a pid file the supervisor wrote (macOS native
    mode; on Windows ``pids.json`` with an "mt5" key), or None when there is no
    usable pid file."""
    try:
        text = path.read_text().strip()
        if IS_WINDOWS and path.suffix.lower() == ".json":
            pid = int(json.loads(text)["mt5"])
        else:
            pid = int(text)
    except (OSError, ValueError, KeyError, TypeError):
        return None
    if IS_WINDOWS:
        return _win_pid_alive(pid)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def terminal_running() -> bool:
    alive = pid_alive(STATE_DIR / ("pids.json" if IS_WINDOWS else "mt5.pid"))
    return alive if alive is not None else process_running("terminal64.exe")


def dir_size(path: Path) -> int:
    total = 0
    if not path.is_dir() or path.is_symlink():
        return 0
    for root, dirs, files in os.walk(path, followlinks=False):
        for name in files:
            fp = os.path.join(root, name)
            try:
                if not os.path.islink(fp):
                    total += os.path.getsize(fp)
            except OSError:
                pass
    return total


def file_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except OSError:
        return 0


def read_json(path: Path) -> Dict[str, Any]:
    try:
        with open(path, encoding="utf-8") as fh:
            data = json.load(fh)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def log_sizes() -> Dict[str, int]:
    """Bytes per log source, rotated copy (<name>.log.1) included."""
    out: Dict[str, int] = {}
    for source, name in LOG_SOURCES.items():
        total = 0
        for p in (LOG_DIR / name, LOG_DIR / (name + ".1")):
            try:
                total += p.stat().st_size
            except OSError:
                pass
        out[source] = total
    return out


def clear_logs() -> Dict[str, Any]:
    """Truncate every log in place (writers append, so they keep working)."""
    freed = sum(log_sizes().values())
    for name in LOG_SOURCES.values():
        for p in (LOG_DIR / name, LOG_DIR / (name + ".1")):
            if p.is_file() and not p.is_symlink():
                try:
                    with open(p, "r+b") as fh:
                        fh.truncate(0)
                except OSError:
                    pass
    return {"cleared": True, "freed_bytes": freed}


def tail(path: Path, lines: int) -> List[str]:
    if not path.is_file():
        return []
    with open(path, "rb") as fh:
        fh.seek(0, os.SEEK_END)
        pos = fh.tell()
        data = b""
        while pos > 0 and data.count(b"\n") <= lines:
            step = min(8192, pos)
            pos -= step
            fh.seek(pos)
            data = fh.read(step) + data
    return data.decode("utf-8", "replace").splitlines()[-lines:]


def cpu_translation() -> str:
    """Mirror of detect_cpu_translation in scripts/cpu-translation.sh. The macOS native
    mode sets SMT5_CPU_TRANSLATION (rosetta2: x86_64 Wine under Rosetta 2)."""
    override = os.environ.get("SMT5_CPU_TRANSLATION", "").strip()
    if override:
        return override
    if IS_WINDOWS:
        return "none"
    try:
        with open("/proc/self/maps", encoding="utf-8", errors="replace") as fh:
            if "rosetta" in fh.read().lower():
                return "rosetta"
    except OSError:
        pass
    try:
        with open("/proc/cpuinfo", encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if line.startswith(("CPU implementer", "CPU architecture")):
                    return "user-mode"
    except OSError:
        pass
    return "none"


CPU_TRANSLATION = cpu_translation()


def read_maxbars() -> Optional[int]:
    cfg = MT5_DIR / "mt5cfg.ini"
    try:
        for raw in cfg.read_text(encoding="utf-8", errors="replace").splitlines():
            if raw.strip().lower().startswith("maxbars="):
                return int(raw.split("=", 1)[1].strip())
    except (OSError, ValueError):
        pass
    return None


# --------------------------------------------------------------------------
# bridge probe — uses the same client as user code
# --------------------------------------------------------------------------
class BridgeProbe:
    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._busy = False
        self._result: Dict[str, Any] = {"ok": False, "error": "not probed yet", "checked_at": None}

    def snapshot(self) -> Dict[str, Any]:
        with self._lock:
            return dict(self._result)

    def _set(self, result: Dict[str, Any]) -> None:
        result["checked_at"] = time.time()
        with self._lock:
            self._result = result

    def loop(self) -> None:
        while True:
            if port_open(BRIDGE_PORT):
                self.run_once()
            else:
                self._set({"ok": False, "error": f"bridge port {BRIDGE_PORT} closed"})
            time.sleep(PROBE_INTERVAL)

    def run_once(self) -> None:
        with self._lock:
            if self._busy:
                return  # a previous probe is still hanging; don't pile up threads
            self._busy = True
        worker = threading.Thread(target=self._probe, name="bridge-probe", daemon=True)
        worker.start()
        worker.join(PROBE_TIMEOUT)
        if worker.is_alive():
            self._set({"ok": False, "error": f"probe timed out after {PROBE_TIMEOUT:.0f}s"})

    def _probe(self) -> None:
        try:
            self._set(self._query())
        except Exception as exc:  # noqa: BLE001 — report anything the bridge throws
            self._set({"ok": False, "error": f"{type(exc).__name__}: {exc}"})
        finally:
            with self._lock:
                self._busy = False

    @staticmethod
    def _query() -> Dict[str, Any]:
        from siliconmetatrader5 import MetaTrader5  # imported lazily: optional dependency

        client = MetaTrader5(host="127.0.0.1", port=BRIDGE_PORT)
        try:
            t0 = time.monotonic()
            client.ping()
            latency_ms = (time.monotonic() - t0) * 1000.0
            initialized = bool(client.initialize())
            ti = client.terminal_info() if initialized else None
            ai = client.account_info() if initialized else None
            result: Dict[str, Any] = {
                "ok": initialized,
                "latency_ms": round(latency_ms, 1),
                "terminal": None,
                "account": None,
            }
            if ti is not None:
                result["terminal"] = {
                    "connected": bool(getattr(ti, "connected", False)),
                    "build": getattr(ti, "build", None),
                    "trade_allowed": bool(getattr(ti, "trade_allowed", False)),
                    # what MT5 actually uses (settings screen compares with .env)
                    "maxbars": _num(getattr(ti, "maxbars", None), int),
                    "dlls_allowed": bool(getattr(ti, "dlls_allowed", False)),
                }
            if ai is not None:
                login = str(getattr(ai, "login", "") or "")
                result["account"] = {
                    "login_masked": ("•••" + login[-3:]) if login else None,
                    "server": getattr(ai, "server", None),
                    "company": getattr(ai, "company", None),
                    "currency": getattr(ai, "currency", None),
                    "trade_mode": TRADE_MODES.get(getattr(ai, "trade_mode", None), "unknown"),
                    "leverage": _num(getattr(ai, "leverage", None), int),
                    "trade_expert": bool(getattr(ai, "trade_expert", True)),
                }
                for key in ("balance", "equity", "profit", "margin", "margin_free", "margin_level"):
                    result["account"][key] = _num(getattr(ai, key, None), float)
                for key in ("positions", "orders"):
                    try:
                        result[key] = _num(getattr(client, f"{key}_total")(), int)
                    except Exception:  # noqa: BLE001 — optional detail
                        result[key] = None
            if not initialized:
                try:
                    result["error"] = f"initialize() failed: {client.last_error()}"
                except Exception:  # noqa: BLE001
                    result["error"] = "initialize() failed"
            return result
        finally:
            client.close()  # never remote_shutdown: other clients share the terminal


def _num(value: Any, kind: Any) -> Any:
    """Plain int/float from an rpyc value (None when missing or not numeric)."""
    try:
        return None if value is None else kind(value)
    except (TypeError, ValueError):
        return None


PROBE = BridgeProbe()


# --------------------------------------------------------------------------
# status
# --------------------------------------------------------------------------
def disk_usage(path: Path) -> Dict[str, Any]:
    probe = path
    while not probe.exists() and probe != probe.parent:
        probe = probe.parent
    if IS_WINDOWS:
        import shutil

        try:
            du = shutil.disk_usage(probe)
        except OSError:
            return {"path": str(path), "free_mb": None, "total_mb": None}
        return {"path": str(path), "free_mb": du.free // (1024 * 1024), "total_mb": du.total // (1024 * 1024)}
    try:
        st = os.statvfs(probe)
    except OSError:
        return {"path": str(path), "free_mb": None, "total_mb": None}
    return {
        "path": str(path),
        "free_mb": st.f_bavail * st.f_frsize // (1024 * 1024),
        "total_mb": st.f_blocks * st.f_frsize // (1024 * 1024),
    }


def compute_status() -> Dict[str, Any]:
    supervisor = read_json(STATE_DIR / "state.json")
    sup_phase = supervisor.get("phase")
    installed = (MT5_DIR / "terminal64.exe").is_file()
    terminal = terminal_running()
    bridge = port_open(BRIDGE_PORT)
    probe = PROBE.snapshot()
    fresh = bool(probe.get("checked_at")) and time.time() - probe["checked_at"] <= 3 * PROBE_INTERVAL + PROBE_TIMEOUT
    connected = bool((probe.get("terminal") or {}).get("connected"))

    if sup_phase in SUPERVISOR_ONLY_PHASES:
        phase = sup_phase
    elif not installed:
        phase = "starting"
    elif not terminal:
        phase = "starting_terminal"
    elif not bridge:
        phase = "starting_bridge"
    elif fresh and probe.get("ok") and connected and probe.get("account"):
        phase = "ready"
    else:
        phase = "waiting_login"

    # A full disk also stops the supervisor from recording its own error, so
    # the API checks it independently and overrides a stale phase.
    disk = disk_usage(STATE_DIR if IS_WINDOWS else WINEPREFIX.parent)
    detail = supervisor.get("detail", "")
    if disk["free_mb"] is not None and disk["free_mb"] < MIN_FREE_MB:
        phase = "error"
        detail = (f"disk almost full: {disk['free_mb']} MB free of {disk['total_mb']} MB in {disk['path']} "
                  + ("(free disk space on the server)" if IS_WINDOWS
                     else "(prune Docker or enlarge the VM disk)"))

    return {
        "version": VERSION,
        "display": DISPLAY,
        "phase": phase,
        "detail": detail,
        "disk": disk,
        "supervisor": supervisor,
        "mt5": {"installed": installed, "running": terminal, "maxbars": read_maxbars()},
        "bridge": {"port": BRIDGE_PORT, "listening": bridge, "probe": probe, "probe_fresh": fresh},
        "host": {"cpu_translation": CPU_TRANSLATION},
        "api": {"mutations_enabled": bool(API_TOKEN), "level": API_LEVEL},
        "time": time.time(),
    }


# --------------------------------------------------------------------------
# data inventory
# --------------------------------------------------------------------------
def _child(parent: Path, name: str) -> Path:
    """parent/name, matched case-insensitively.

    Windows programs under Wine may create "Bases", "History", "Ticks" … while
    the Linux filesystem is case-sensitive.
    """
    exact = parent / name
    if exact.exists():
        return exact
    try:
        for entry in parent.iterdir():
            if entry.name.lower() == name.lower():
                return entry
    except OSError:
        pass
    return exact


def _read_origin(folder: Path) -> str:
    """origin.txt in an MT5 data folder holds the install path (UTF-16)."""
    try:
        raw = (folder / "origin.txt").read_bytes()
    except OSError:
        return ""
    for enc in ("utf-16", "utf-8"):
        try:
            return raw.decode(enc).strip()
        except UnicodeDecodeError:
            continue
    return ""


def mt5_data_dir() -> Path:
    """Folder where MT5 keeps bases/ and Tester/.

    Portable mode uses the install folder, but under Wine the terminal may still
    use the per-user data folder
    ``users/<user>/AppData/Roaming/MetaQuotes/Terminal/<id>/``. Override with
    SMT5_MT5_DATA_DIR.
    """
    override = os.environ.get("SMT5_MT5_DATA_DIR")
    if override:
        return Path(override)
    if IS_WINDOWS or _child(MT5_DIR, "bases").is_dir():
        return MT5_DIR  # Windows: portable install, the terminal data dir is the install dir
    candidates = []
    for term in (WINEPREFIX / "drive_c" / "users").glob("*/AppData/Roaming/MetaQuotes/Terminal/*"):
        if not term.is_dir() or term.is_symlink() or not _child(term, "bases").is_dir():
            continue
        origin = _read_origin(term).lower()
        mine = "metatrader 5" in origin or not origin
        try:
            mtime = _child(term, "bases").stat().st_mtime
        except OSError:
            mtime = 0.0
        candidates.append((mine, mtime, term))
    if candidates:
        return max(candidates, key=lambda c: (c[0], c[1]))[2]
    return MT5_DIR


def bases_root() -> Path:
    return _child(mt5_data_dir(), "bases")


def _subdirs(path: Path) -> Iterable[Path]:
    if not path.is_dir():
        return []
    return sorted(p for p in path.iterdir() if p.is_dir() and not p.is_symlink())


def _scan_history(history: Path) -> List[Dict[str, Any]]:
    out = []
    for sym in _subdirs(history):
        years: Dict[str, int] = {}
        for f in sym.iterdir():
            m = HCC_RE.match(f.name)
            if m and f.is_file() and not f.is_symlink():
                years[m.group(1)] = file_size(f)
        cache = dir_size(_child(sym, "cache"))
        total = dir_size(sym)
        if not total and not years:
            continue  # empty folder left behind (e.g. after a purge)
        out.append({
            "symbol": sym.name,
            "bytes": total,
            "years": dict(sorted(years.items())),
            "cache_bytes": cache,
            "other_bytes": max(total - cache - sum(years.values()), 0),
        })
    return sorted(out, key=lambda s: s["bytes"], reverse=True)


def _scan_ticks(ticks: Path) -> List[Dict[str, Any]]:
    out = []
    for sym in _subdirs(ticks):
        months: Dict[str, int] = {}
        for f in sym.iterdir():
            m = TKC_RE.match(f.name)
            if m and f.is_file() and not f.is_symlink():
                months[m.group(1)] = file_size(f)
        total = dir_size(sym)
        if not total and not months:
            continue  # empty folder left behind (e.g. after a purge)
        # Monthly YYYYMM.tkc files appear once tick history is downloaded
        # (copy_ticks_*, tick-based tests). Live terminals keep a rolling
        # ticks.dat cache per symbol, reported as "other_bytes".
        out.append({
            "symbol": sym.name,
            "bytes": total,
            "months": dict(sorted(months.items())),
            "other_bytes": max(total - sum(months.values()), 0),
        })
    return sorted(out, key=lambda s: s["bytes"], reverse=True)


def inventory() -> Dict[str, Any]:
    servers = []
    for sdir in _subdirs(bases_root()):
        history = _scan_history(_child(sdir, "history"))
        ticks = _scan_ticks(_child(sdir, "ticks"))
        if not history and not ticks:
            continue
        servers.append({
            "name": sdir.name,
            "bytes": dir_size(sdir),
            "history_bytes": sum(s["bytes"] for s in history),
            "ticks_bytes": sum(s["bytes"] for s in ticks),
            "cache_bytes": sum(s["cache_bytes"] for s in history),
            "history": history,
            "ticks": ticks,
        })
    data_dir = mt5_data_dir()
    tester = dir_size(_child(data_dir, "Tester"))
    bases = dir_size(bases_root())
    return {
        "root": str(data_dir),
        "install_dir": str(MT5_DIR),
        "exists": data_dir.is_dir(),
        "total_bytes": bases + tester,
        "bases_bytes": bases,
        "tester_bytes": tester,
        "servers": servers,
        "scanned_at": time.time(),
    }


# --------------------------------------------------------------------------
# purge planning + execution
# --------------------------------------------------------------------------
def _name(value: Any, field: str) -> str:
    if value is None or value == "*":
        return "*"
    if not isinstance(value, str) or not NAME_RE.match(value):
        raise ApiError(HTTPStatus.BAD_REQUEST, f"invalid {field}: {value!r}")
    return value


def _pick(parent: Path, name: str) -> List[Path]:
    if name == "*":
        return list(_subdirs(parent))
    candidate = parent / name
    return [candidate] if candidate.is_dir() and not candidate.is_symlink() else []


def _int_list(value: Any, field: str, digits: int) -> List[int]:
    if not isinstance(value, list) or not value or len(value) > 1000:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"{field} must be a non-empty list")
    out = []
    for v in value:
        s = str(v)
        if not (s.isdigit() and len(s) == digits):
            raise ApiError(HTTPStatus.BAD_REQUEST, f"invalid value in {field}: {v!r}")
        out.append(int(s))
    return out


def _selector(item: Dict[str, Any], list_key: str, before_key: str, digits: int):
    """Return a predicate on the numeric file stem (year or yyyymm)."""
    given = [k for k in (list_key, before_key, "all") if k in item]
    if len(given) != 1:
        raise ApiError(HTTPStatus.BAD_REQUEST,
                       f"{item.get('kind')} item needs exactly one of {list_key}, {before_key}, all")
    key = given[0]
    if key == "all":
        if item["all"] is not True:
            raise ApiError(HTTPStatus.BAD_REQUEST, "all must be true")
        return lambda _n: True
    if key == list_key:
        wanted = set(_int_list(item[list_key], list_key, digits))
        return lambda n: n in wanted
    limit = _int_list([item[before_key]], before_key, digits)[0]
    return lambda n: n < limit


def _files_in(path: Path) -> List[Path]:
    out = []
    if path.is_dir() and not path.is_symlink():
        for root, _dirs, files in os.walk(path, followlinks=False):
            out.extend(Path(root) / f for f in files)
    return out


def plan_purge(spec: Dict[str, Any]) -> List[Path]:
    items = spec.get("items")
    if not isinstance(items, list) or not items or len(items) > MAX_ITEMS:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"items must be a list of 1..{MAX_ITEMS} objects")

    root = bases_root()
    planned: List[Path] = []
    for item in items:
        if not isinstance(item, dict):
            raise ApiError(HTTPStatus.BAD_REQUEST, "each item must be an object")
        kind = item.get("kind")
        if kind == "tester":
            tester = _child(mt5_data_dir(), "Tester")
            for agent in _subdirs(tester):
                if agent.name.lower().startswith("agent-") or agent.name.lower() == "cache":
                    planned.extend(_files_in(agent))
            continue
        if kind not in ("history", "ticks", "cache"):
            raise ApiError(HTTPStatus.BAD_REQUEST, f"unknown kind: {kind!r}")

        server = _name(item.get("server"), "server")
        symbol = _name(item.get("symbol"), "symbol")
        if kind == "history":
            match = _selector(item, "years", "before_year", 4)
        elif kind == "ticks":
            match = _selector(item, "months", "before_month", 6)
        else:
            match = None

        for sdir in _pick(root, server):
            sub = "ticks" if kind == "ticks" else "history"
            for sym in _pick(_child(sdir, sub), symbol):
                if kind == "cache":
                    planned.extend(_files_in(_child(sym, "cache")))
                    continue
                if kind == "ticks" and item.get("all") is True:
                    # "all" ticks also removes the rolling ticks.dat cache.
                    planned.extend(f for f in sym.iterdir() if f.is_file())
                    continue
                pattern = TKC_RE if kind == "ticks" else HCC_RE
                for f in sym.iterdir():
                    m = pattern.match(f.name)
                    if m and f.is_file() and match(int(m.group(1))):
                        planned.append(f)

    # Final safety net: regular files strictly inside the MT5 data folders only.
    allowed_roots = [root.resolve(), _child(mt5_data_dir(), "Tester").resolve()]
    safe: List[Path] = []
    seen = set()
    for p in planned:
        if p.is_symlink() or not p.is_file():
            continue
        rp = p.resolve()
        if not any(rp != r and rp.is_relative_to(r) for r in allowed_roots):
            continue
        if rp not in seen:
            seen.add(rp)
            safe.append(p)
    return safe


def _prune_empty_dirs(paths: Iterable[Path]) -> None:
    """Remove now-empty cache/tester subdirectories (never the data roots)."""
    keep = {bases_root().resolve(), _child(mt5_data_dir(), "Tester").resolve()}
    for d in sorted({p.parent for p in paths}, key=lambda x: len(x.parts), reverse=True):
        cur = d
        while cur.resolve() not in keep and cur.name.lower() != "history" and cur.name.lower() != "ticks":
            try:
                cur.rmdir()
            except OSError:
                break
            cur = cur.parent


_maintenance_lock = threading.Lock()


def with_terminal_stopped(reason: str, action):
    """Ask the supervisor to stop MT5, run ``action``, then let it restart MT5."""
    if not _maintenance_lock.acquire(blocking=False):
        raise ApiError(HTTPStatus.CONFLICT, "another maintenance operation is running")
    request = STATE_DIR / "maintenance.request"
    ready = STATE_DIR / "maintenance.ready"
    try:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        request.write_text(reason[:200], encoding="utf-8")
        deadline = time.monotonic() + MAINTENANCE_TIMEOUT
        while not ready.exists():
            if time.monotonic() > deadline:
                raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE,
                               f"supervisor did not stop MT5 within {MAINTENANCE_TIMEOUT:.0f}s; nothing was deleted")
            time.sleep(0.5)
        return action()
    finally:
        try:
            request.unlink()
        except FileNotFoundError:
            pass
        _maintenance_lock.release()


def purge(spec: Dict[str, Any]) -> Dict[str, Any]:
    dry_run = spec.get("dry_run", True)
    if not isinstance(dry_run, bool):
        raise ApiError(HTTPStatus.BAD_REQUEST, "dry_run must be a boolean")
    data_dir = mt5_data_dir()
    files = plan_purge(spec)
    sizes = {p: file_size(p) for p in files}
    summary = {
        "dry_run": dry_run,
        "files": len(files),
        "bytes": sum(sizes.values()),
        "sample": [str(p.relative_to(data_dir)) for p in files[:50]],
    }
    if dry_run or not files:
        summary["deleted"] = False
        return summary

    def _delete() -> Dict[str, Any]:
        removed, freed, failed = 0, 0, []
        for p in files:
            try:
                p.unlink()
                removed += 1
                freed += sizes[p]
            except OSError as exc:
                failed.append(f"{p.relative_to(data_dir)}: {exc.strerror}")
        _prune_empty_dirs(files)
        return {"removed": removed, "freed_bytes": freed, "failed": failed[:50]}

    result = with_terminal_stopped(f"data purge: {len(files)} files", _delete)
    summary.update(result)
    summary["deleted"] = True
    return summary


_datatest_lock = threading.Lock()
DATATEST_TIMEOUT = float(os.environ.get("SMT5_DATATEST_TIMEOUT", "150"))


def run_datatest(symbol: str, tf: str, count: int, tick_seconds: float, ops_factory=None) -> Dict[str, Any]:
    import datatest

    datatest.validate(symbol, tf, count, tick_seconds)  # 400 before touching the bridge
    if not port_open(BRIDGE_PORT):
        return {"symbol": symbol, "timeframe": tf, "checks": [], "verdict": "error",
                "message": f"bridge port {BRIDGE_PORT} is closed (runtime still starting?)"}
    if not _datatest_lock.acquire(blocking=False):
        raise ApiError(HTTPStatus.CONFLICT, "a data test is already running")
    box: Dict[str, Any] = {}

    def work() -> None:
        ops = None
        try:
            ops = (ops_factory or (lambda: datatest.BridgeOps("127.0.0.1", BRIDGE_PORT)))()
            box["result"] = datatest.run(ops, symbol, tf, count, tick_seconds)
        except Exception as exc:  # noqa: BLE001
            box["result"] = {"symbol": symbol, "timeframe": tf, "checks": [], "verdict": "error",
                             "message": f"{type(exc).__name__}: {exc}"}
        finally:
            if ops is not None:
                try:
                    ops.close()
                except Exception:  # noqa: BLE001
                    pass

    try:
        worker = threading.Thread(target=work, name="datatest", daemon=True)
        worker.start()
        worker.join(DATATEST_TIMEOUT)
        if worker.is_alive():
            return {"symbol": symbol, "timeframe": tf, "checks": [], "verdict": "error",
                    "message": f"data test timed out after {DATATEST_TIMEOUT:.0f}s"}
        return box["result"]
    finally:
        _datatest_lock.release()


_strategy_lock = threading.Lock()
STRATEGY_TIMEOUT = float(os.environ.get("SMT5_STRATEGY_TIMEOUT", "90"))
STRATEGY_CHART_BARS = 300


def strategy_payload(symbol: str, tf: str, rows: List[Any], digits: int, params: Any) -> Dict[str, Any]:
    """Backtest on the CLOSED bars (the forming last row is dropped) + chart tail."""
    import supertrend_strategy as sts

    bars = sts.rows_to_bars(rows)
    forming = bars[-1].time if bars else None
    closed = bars[:-1]
    res = sts.backtest(closed, params)
    tail = STRATEGY_CHART_BARS
    stats = dict(res.stats)
    stats["equity"] = stats["equity"][-500:]
    return {
        "symbol": symbol, "timeframe": tf, "digits": digits, "params": res.params,
        "count": len(closed), "from": closed[0].time if closed else None,
        "last_closed": closed[-1].time if closed else None, "forming": forming,
        "trend": res.trend[-1] if res.trend else 0, "line_now": res.line[-1] if res.line else None,
        "chart": {"bars": [[b.time, b.open, b.high, b.low, b.close] for b in closed[-tail:]],
                  "line": res.line[-tail:], "trend": res.trend[-tail:]},
        "signals": [vars(x) for x in res.signals[-300:]],
        "trades": [vars(x) for x in res.trades[-300:]],
        "stats": stats,
    }


STRATEGY_RETRY_WAIT = float(os.environ.get("SMT5_STRATEGY_RETRY_WAIT", "1.5"))


def fetch_bars(ops: Any, symbol: str, tf: str, count: int) -> List[Any]:
    """copy_rates_from_pos that copes with MT5's empty answers.

    MT5 returns nothing (1) while it is still building a timeframe it was not
    asked for before (D1, H4 …) and (2) when the request is longer than the
    history the broker offers. Wait and ask again, then ask for less.
    """
    n = count
    for attempt in range(8):
        rows = ops.rates_from_pos(symbol, tf, n)
        if rows:
            return rows
        if attempt < 2:
            time.sleep(STRATEGY_RETRY_WAIT)
        elif n > 101:
            n = max(101, n // 2)
        else:
            break
    return []


def run_supertrend(symbol: str, tf: str, period: int, multiplier: float, count: int, ops_factory=None) -> Dict[str, Any]:
    import datatest
    import supertrend_strategy as sts

    try:
        datatest.validate(symbol, tf, 10, 0)
        params = sts.Params(period, multiplier)
        params.check()
    except ValueError as exc:
        raise ApiError(HTTPStatus.BAD_REQUEST, str(exc)) from None
    if not 100 <= count <= 20_000:
        raise ApiError(HTTPStatus.BAD_REQUEST, "bars must be between 100 and 20000")
    if not port_open(BRIDGE_PORT):
        raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE, f"bridge port {BRIDGE_PORT} is closed (runtime still starting?)")
    if not _strategy_lock.acquire(blocking=False):
        raise ApiError(HTTPStatus.CONFLICT, "a strategy run is already in progress")
    box: Dict[str, Any] = {}

    def work() -> None:
        ops = None
        try:
            ops = (ops_factory or (lambda: datatest.BridgeOps("127.0.0.1", BRIDGE_PORT)))()
            ok, err = ops.initialize()
            if not ok:
                raise RuntimeError(f"initialize() failed: {err}")
            if not ops.select(symbol):
                raise RuntimeError(f"{symbol}: not available at this broker")
            info = ops.symbol_info(symbol)
            rows = fetch_bars(ops, symbol, tf, count + 1)
            if not rows:
                raise RuntimeError(f"no bars for {symbol} {tf} yet (history still downloading?)")
            box["result"] = strategy_payload(symbol, tf, rows, int(info[0]) if info else 5, params)
            box["result"]["requested"] = count
        except Exception as exc:  # noqa: BLE001
            box["error"] = f"{type(exc).__name__}: {exc}"
        finally:
            if ops is not None:
                try:
                    ops.close()
                except Exception:  # noqa: BLE001
                    pass

    try:
        worker = threading.Thread(target=work, name="strategy", daemon=True)
        worker.start()
        worker.join(STRATEGY_TIMEOUT)
        if worker.is_alive():
            raise ApiError(HTTPStatus.GATEWAY_TIMEOUT, f"strategy run timed out after {STRATEGY_TIMEOUT:.0f}s")
        if "error" in box:
            raise ApiError(HTTPStatus.BAD_GATEWAY, box["error"])
        return box["result"]
    finally:
        _strategy_lock.release()


_account_lock = threading.Lock()
ACCOUNT_TIMEOUT = float(os.environ.get("SMT5_ACCOUNT_TIMEOUT", "75"))


def _account_fields(ai: Any) -> Optional[Dict[str, Any]]:
    if ai is None or not getattr(ai, "login", 0):
        return None
    return {"login": int(ai.login), "server": str(ai.server), "name": str(ai.name or ""), "company": str(ai.company or ""),
            "currency": str(ai.currency or ""), "leverage": int(ai.leverage or 0),
            "trade_mode": {0: "demo", 1: "contest", 2: "real"}.get(int(ai.trade_mode), "unknown")}


def _with_client(name: str, fn, ops_factory=None) -> Dict[str, Any]:
    """Run fn(client) on a fresh bridge connection, bounded by ACCOUNT_TIMEOUT."""
    import datatest

    if not port_open(BRIDGE_PORT):
        raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE, f"bridge port {BRIDGE_PORT} is closed (runtime still starting?)")
    if not _account_lock.acquire(blocking=False):
        raise ApiError(HTTPStatus.CONFLICT, "an account operation is already running")
    box: Dict[str, Any] = {}

    def work() -> None:
        ops = None
        try:
            ops = (ops_factory or (lambda: datatest.BridgeOps("127.0.0.1", BRIDGE_PORT)))()
            box["result"] = fn(ops.c)
        except Exception as exc:  # noqa: BLE001
            box["error"] = f"{type(exc).__name__}: {exc}"
        finally:
            if ops is not None:
                try:
                    ops.close()
                except Exception:  # noqa: BLE001
                    pass

    try:
        worker = threading.Thread(target=work, name=name, daemon=True)
        worker.start()
        worker.join(ACCOUNT_TIMEOUT)
        if worker.is_alive():
            raise ApiError(HTTPStatus.GATEWAY_TIMEOUT, f"MT5 did not answer within {ACCOUNT_TIMEOUT:.0f}s")
        if "error" in box:
            raise ApiError(HTTPStatus.BAD_GATEWAY, box["error"])
        return box["result"]
    finally:
        _account_lock.release()


def run_account(ops_factory=None) -> Dict[str, Any]:
    """The account MT5 is logged in to right now (MT5 is the source of truth)."""
    def fn(c):
        if not c.initialize():
            return {"account": None, "connected": False, "error": str(c.last_error())}
        ti = c.terminal_info()
        return {"account": _account_fields(c.account_info()), "connected": bool(ti and ti.connected)}
    return _with_client("account", fn, ops_factory)


# MT5 deal types (TRADE_DEAL_TYPE / DEAL_ENTRY)
DEAL_BUY, DEAL_SELL, DEAL_BALANCE, DEAL_CREDIT = 0, 1, 2, 3
ENTRY_IN = 0
HISTORY_RANGES = {"today": None, "7d": 7, "30d": 30, "90d": 90, "all": 0}


def summarize_history(deals: List[Any], balance: float, equity: float, since: int, server_now: int) -> Dict[str, Any]:
    """Balance curve and P/L of ONE account from its MT5 deal history.

    deals: (time, type, entry, profit, commission, swap, fee, symbol, volume, position_id).
    Trading P/L counts buy/sell deals only (profit + commission + swap + fee);
    deposits and withdrawals (balance deals) are reported apart and never as
    profit; credit does not change the balance.
    """
    deals = sorted((d for d in deals if d[0] >= since), key=lambda d: d[0])
    trading = deposits = withdrawals = other = 0.0
    wins = losses = trades = 0
    by_symbol: Dict[str, float] = {}
    changes = []
    for t, typ, entry, profit, comm, swap, fee, sym, vol, pos in deals:
        if typ == DEAL_CREDIT:
            continue
        delta = profit + comm + swap + fee
        if typ in (DEAL_BUY, DEAL_SELL):
            trading += delta
            if entry != ENTRY_IN:                      # a deal that closes (part of) a position
                trades += 1
                wins += profit > 0
                losses += profit < 0
            if sym:
                by_symbol[sym] = by_symbol.get(sym, 0.0) + delta
        elif typ == DEAL_BALANCE:
            if profit >= 0:
                deposits += profit
            else:
                withdrawals += profit
        else:
            other += delta
        changes.append((t, delta, typ))
    start = balance - sum(c[1] for c in changes)
    curve, run = [[since if since else (changes[0][0] if changes else server_now), round(start, 2), -1]], start
    for t, delta, typ in changes:
        run += delta
        curve.append([t, round(run, 2), typ])
    return {
        "from": since, "to": server_now, "start_balance": round(start, 2), "balance": balance, "equity": equity,
        "floating": round(equity - balance, 2), "trading_pl": round(trading, 2), "deposits": round(deposits, 2),
        "withdrawals": round(withdrawals, 2), "other": round(other, 2), "trades": trades, "wins": wins, "losses": losses,
        "deals": len(changes), "curve": curve[-2000:],
        "by_symbol": sorted(([s, round(v, 2)] for s, v in by_symbol.items()), key=lambda x: x[1]),
    }


def run_account_history(rng: str, ops_factory=None) -> Dict[str, Any]:
    """Deal history of the account MT5 is logged in to right now."""
    if rng not in HISTORY_RANGES:
        raise ApiError(HTTPStatus.BAD_REQUEST, f"range must be one of {sorted(HISTORY_RANGES)}")
    import datatest

    if not port_open(BRIDGE_PORT):
        raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE, f"bridge port {BRIDGE_PORT} is closed (runtime still starting?)")
    if not _account_lock.acquire(blocking=False):
        raise ApiError(HTTPStatus.CONFLICT, "an account operation is already running")
    box: Dict[str, Any] = {}

    def work() -> None:
        ops = None
        try:
            ops = (ops_factory or (lambda: datatest.BridgeOps("127.0.0.1", BRIDGE_PORT)))()
            c = ops.c
            if not c.initialize():
                raise RuntimeError(f"initialize() failed: {c.last_error()}")
            acc = _account_fields(c.account_info())
            if acc is None:
                box["result"] = {"account": None}
                return
            ai = c.account_info()
            now = ops.server_now()
            days = HISTORY_RANGES[rng]
            since = (now // 86400) * 86400 if days is None else (now - days * 86400 if days else 0)
            deals = ops.deals(max(0, since - 86400) if since else 946684800, now + 3 * 86400)
            res = summarize_history(deals, float(ai.balance), float(ai.equity), since, now)
            res.update({"account": acc, "range": rng, "currency": acc["currency"]})
            box["result"] = res
        except Exception as exc:  # noqa: BLE001
            box["error"] = f"{type(exc).__name__}: {exc}"
        finally:
            if ops is not None:
                try:
                    ops.close()
                except Exception:  # noqa: BLE001
                    pass

    try:
        worker = threading.Thread(target=work, name="history", daemon=True)
        worker.start()
        worker.join(ACCOUNT_TIMEOUT)
        if worker.is_alive():
            raise ApiError(HTTPStatus.GATEWAY_TIMEOUT, f"MT5 did not answer within {ACCOUNT_TIMEOUT:.0f}s")
        if "error" in box:
            raise ApiError(HTTPStatus.BAD_GATEWAY, box["error"])
        return box["result"]
    finally:
        _account_lock.release()


BENCH_TIMEOUT = float(os.environ.get("SMT5_BENCH_TIMEOUT", "300"))


def boot_timings() -> Dict[str, Any]:
    """When this runtime started (container PID 1, or SMT5_STARTED_AT from the
    macOS supervisor) and when the supervisor first reported MT5 + bridge
    running (state.json `updated` of the `running` phase)."""
    out: Dict[str, Any] = {"container_started": None, "running_at": None, "boot_s": None}
    started = os.environ.get("SMT5_STARTED_AT", "")
    if started.replace(".", "", 1).isdigit():
        out["container_started"] = float(started)
    elif IS_WINDOWS:
        try:
            uptime = _kernel32().GetTickCount64() / 1000
            out["container_started"] = round(time.time() - uptime, 1)
        except (OSError, AttributeError, ValueError, TypeError):
            pass
    else:
        try:
            ticks = os.sysconf("SC_CLK_TCK")
            start_ticks = int(Path("/proc/1/stat").read_text().rsplit(")", 1)[1].split()[19])
            uptime = float(Path("/proc/uptime").read_text().split()[0])
            out["container_started"] = round(time.time() - uptime + start_ticks / ticks, 1)
        except (OSError, ValueError, IndexError):
            pass
    state = read_json(STATE_DIR / "state.json") or {}
    if state.get("phase") == "running" and state.get("updated"):
        out["running_at"] = state["updated"]
        if out["container_started"]:
            out["boot_s"] = round(float(state["updated"]) - out["container_started"], 1)
    return out


_bench_lock = threading.Lock()


def run_bench(symbol: str, bars: int, ops_factory=None) -> Dict[str, Any]:
    import datatest

    datatest.validate(symbol, "M1", 100, 0)
    if not 1_000 <= bars <= 1_000_000:
        raise ApiError(HTTPStatus.BAD_REQUEST, "bars must be between 1000 and 1000000")
    if not port_open(BRIDGE_PORT):
        raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE, f"bridge port {BRIDGE_PORT} is closed (runtime still starting?)")
    if not _bench_lock.acquire(blocking=False):
        raise ApiError(HTTPStatus.CONFLICT, "a speed test is already running")
    box: Dict[str, Any] = {}

    def work() -> None:
        ops = None
        try:
            ops = (ops_factory or (lambda: datatest.BridgeOps("127.0.0.1", BRIDGE_PORT)))()
            box["result"] = datatest.bench(ops, symbol, bars)
        except Exception as exc:  # noqa: BLE001
            box["result"] = {"ok": False, "error": f"{type(exc).__name__}: {exc}"}
        finally:
            if ops is not None:
                try:
                    ops.close()
                except Exception:  # noqa: BLE001
                    pass

    try:
        worker = threading.Thread(target=work, name="bench", daemon=True)
        worker.start()
        worker.join(BENCH_TIMEOUT)
        result = box.get("result") or {"ok": False, "error": f"speed test timed out after {BENCH_TIMEOUT:.0f}s"}
    finally:
        _bench_lock.release()
    result.update(boot_timings())
    result["wine_flavor"] = os.environ.get("SMT5_WINE_FLAVOR", "winehq")
    result["wine_version"] = os.environ.get("WINE_VERSION", "")
    result["cpu_translation"] = CPU_TRANSLATION
    result["at"] = time.time()
    return result


SYMBOLS_TIMEOUT = float(os.environ.get("SMT5_SYMBOLS_TIMEOUT", "30"))


def run_market_watch(ops_factory=None) -> Dict[str, Any]:
    """Market Watch symbol names through the bridge (for the app's symbol pickers)."""
    import datatest

    if not port_open(BRIDGE_PORT):
        raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE, f"bridge port {BRIDGE_PORT} is closed (runtime still starting?)")
    box: Dict[str, Any] = {}

    def work() -> None:
        ops = None
        try:
            ops = (ops_factory or (lambda: datatest.BridgeOps("127.0.0.1", BRIDGE_PORT)))()
            box["result"] = datatest.market_watch(ops)
        except Exception as exc:  # noqa: BLE001
            box["error"] = f"{type(exc).__name__}: {exc}"
        finally:
            if ops is not None:
                try:
                    ops.close()
                except Exception:  # noqa: BLE001
                    pass

    worker = threading.Thread(target=work, name="market-watch", daemon=True)
    worker.start()
    worker.join(SYMBOLS_TIMEOUT)
    if worker.is_alive():
        raise ApiError(HTTPStatus.GATEWAY_TIMEOUT, f"bridge did not answer within {SYMBOLS_TIMEOUT:.0f}s")
    if "error" in box:
        raise ApiError(HTTPStatus.BAD_GATEWAY, box["error"])
    return box["result"]


def request_restart(target: str) -> Dict[str, Any]:
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    (STATE_DIR / f"restart-{target}.request").write_text(str(time.time()), encoding="utf-8")
    return {"accepted": True, "target": target}


# --------------------------------------------------------------------------
# Clipboard (Server mode: the desktop app syncs its clipboard with MT5's
# through the SSH tunnel). Windows: the Win32 clipboard of the session this
# process runs in (the supervisor starts it in the MT5 desktop session).
# Linux: xclip on the runtime's X display.
# --------------------------------------------------------------------------

CLIP_MAX = 1 << 20  # bytes of UTF-8, as the desktop app's limit
CLIP_BODY_MAX = 8 * CLIP_MAX  # JSON escaping can grow the text
CF_UNICODETEXT = 13
GMEM_MOVEABLE = 0x0002
_USER32: Any = None


def _user32() -> Any:
    global _USER32
    if _USER32 is None:
        import ctypes
        from ctypes import wintypes

        u32 = ctypes.WinDLL("user32", use_last_error=True)  # type: ignore[attr-defined]
        u32.OpenClipboard.argtypes = [wintypes.HWND]
        u32.OpenClipboard.restype = wintypes.BOOL
        u32.CloseClipboard.argtypes = []
        u32.CloseClipboard.restype = wintypes.BOOL
        u32.EmptyClipboard.argtypes = []
        u32.EmptyClipboard.restype = wintypes.BOOL
        u32.GetClipboardData.argtypes = [wintypes.UINT]
        u32.GetClipboardData.restype = wintypes.HANDLE
        u32.SetClipboardData.argtypes = [wintypes.UINT, wintypes.HANDLE]
        u32.SetClipboardData.restype = wintypes.HANDLE
        u32.CreateWindowExW.argtypes = [wintypes.DWORD, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD, ctypes.c_int, ctypes.c_int,
                                        ctypes.c_int, ctypes.c_int, wintypes.HWND, wintypes.HMENU, wintypes.HINSTANCE, wintypes.LPVOID]
        u32.CreateWindowExW.restype = wintypes.HWND
        u32.DestroyWindow.argtypes = [wintypes.HWND]
        u32.DestroyWindow.restype = wintypes.BOOL
        k32 = _kernel32()
        k32.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
        k32.GlobalAlloc.restype = wintypes.HGLOBAL
        k32.GlobalLock.argtypes = [wintypes.HGLOBAL]
        k32.GlobalLock.restype = ctypes.c_void_p
        k32.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
        k32.GlobalUnlock.restype = wintypes.BOOL
        k32.GlobalFree.argtypes = [wintypes.HGLOBAL]
        k32.GlobalFree.restype = wintypes.HGLOBAL
        _USER32 = u32
    return _USER32


class _WinClipboard:
    """OpenClipboard … CloseClipboard with a hidden window of this thread as the
    owner: with a NULL owner, EmptyClipboard makes SetClipboardData fail."""

    def __enter__(self) -> "_WinClipboard":
        u32 = _user32()
        self.hwnd = u32.CreateWindowExW(0, "STATIC", None, 0, 0, 0, 0, 0, None, None, None, None)
        for _ in range(10):  # another program may hold it for a moment
            if u32.OpenClipboard(self.hwnd):
                return self
            time.sleep(0.05)
        if self.hwnd:
            u32.DestroyWindow(self.hwnd)
        raise ApiError(HTTPStatus.SERVICE_UNAVAILABLE, "clipboard is busy")

    def __exit__(self, *exc: Any) -> None:
        u32 = _user32()
        u32.CloseClipboard()
        if self.hwnd:
            u32.DestroyWindow(self.hwnd)


def _win_clip_get() -> Optional[str]:
    import ctypes

    u32, k32 = _user32(), _kernel32()
    with _WinClipboard():
        h = u32.GetClipboardData(CF_UNICODETEXT)
        if not h:
            return None
        ptr = k32.GlobalLock(h)
        if not ptr:
            return None
        try:
            return ctypes.wstring_at(ptr)
        finally:
            k32.GlobalUnlock(h)


def _win_clip_set(text: str) -> None:
    import ctypes

    u32, k32 = _user32(), _kernel32()
    data = (text + "\0").encode("utf-16-le")
    h = k32.GlobalAlloc(GMEM_MOVEABLE, len(data))
    if not h:
        raise ApiError(HTTPStatus.INTERNAL_SERVER_ERROR, "GlobalAlloc failed")
    ptr = k32.GlobalLock(h)
    if not ptr:
        k32.GlobalFree(h)
        raise ApiError(HTTPStatus.INTERNAL_SERVER_ERROR, "GlobalLock failed")
    ctypes.memmove(ptr, data, len(data))
    k32.GlobalUnlock(h)
    try:
        with _WinClipboard():
            u32.EmptyClipboard()
            if not u32.SetClipboardData(CF_UNICODETEXT, h):
                raise ApiError(HTTPStatus.INTERNAL_SERVER_ERROR, "SetClipboardData failed")
    except ApiError:
        k32.GlobalFree(h)  # still ours when the clipboard did not take it
        raise


def _xclip(args: List[str], data: Optional[bytes] = None) -> bytes:
    import subprocess

    env = dict(os.environ, DISPLAY=os.environ.get("DISPLAY") or ":100")
    try:
        r = subprocess.run(["xclip", "-selection", "clipboard", *args], input=data, capture_output=True, timeout=5, env=env)
    except FileNotFoundError:
        raise ApiError(HTTPStatus.NOT_IMPLEMENTED, "xclip is not installed on the server (sudo apt install xclip)") from None
    except subprocess.TimeoutExpired:
        raise ApiError(HTTPStatus.GATEWAY_TIMEOUT, "xclip did not answer") from None
    if data is None and r.returncode != 0:
        return b""  # empty clipboard / no owner yet
    if data is not None and r.returncode != 0:
        raise ApiError(HTTPStatus.INTERNAL_SERVER_ERROR, r.stderr.decode("utf-8", "replace").strip() or "xclip failed")
    return r.stdout


def clipboard_get() -> Dict[str, Any]:
    if IS_WINDOWS:
        text = _win_clip_get()
    else:
        raw = _xclip(["-o", "-t", "UTF8_STRING"]) or _xclip(["-o"])
        text = raw.decode("utf-8", "replace") if raw else None
    if text is not None and (not text or len(text.encode("utf-8")) > CLIP_MAX):
        text = None
    return {"text": text}


def clipboard_set(body: Dict[str, Any]) -> Dict[str, Any]:
    text = body.get("text")
    if not isinstance(text, str):
        raise ApiError(HTTPStatus.BAD_REQUEST, "text must be a string")
    if len(text.encode("utf-8")) > CLIP_MAX:
        raise ApiError(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "text too large")
    if IS_WINDOWS:
        _win_clip_set(text)
    else:
        _xclip(["-i"], text.encode("utf-8"))
    return {"ok": True, "chars": len(text)}


# --------------------------------------------------------------------------
# HTTP
# --------------------------------------------------------------------------
class Handler(BaseHTTPRequestHandler):
    server_version = f"siliconmt5-status/{VERSION}"
    protocol_version = "HTTP/1.0"  # one request per connection: no half-read bodies on errors

    # ---- plumbing ----------------------------------------------------
    def log_message(self, fmt: str, *args: Any) -> None:  # concise access log
        sys.stderr.write(f"{time.strftime('%Y-%m-%d %H:%M:%S')} [status] {self.address_string()} {fmt % args}\n")

    def _send(self, status: HTTPStatus, payload: Dict[str, Any]) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def _check_host(self) -> None:
        # Blocks DNS-rebinding pages from reading the API through a browser.
        host = (self.headers.get("Host") or "").strip().lower()
        if host.startswith("["):
            hostname = host.split("]")[0] + "]"
        else:
            hostname = host.rsplit(":", 1)[0] if ":" in host else host
        if hostname not in ALLOWED_HOSTS:
            raise ApiError(HTTPStatus.FORBIDDEN, "host not allowed")

    def _check_token(self) -> None:
        if not API_TOKEN:
            raise ApiError(HTTPStatus.FORBIDDEN, "SMT5_API_TOKEN is not set; mutating endpoints are disabled")
        header = self.headers.get("Authorization", "")
        token = header[7:] if header.startswith("Bearer ") else ""
        if not hmac.compare_digest(token.encode(), API_TOKEN.encode()):
            raise ApiError(HTTPStatus.UNAUTHORIZED, "missing or invalid bearer token")

    def _json_body(self, limit: int = MAX_BODY) -> Dict[str, Any]:
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            raise ApiError(HTTPStatus.BAD_REQUEST, "invalid Content-Length") from None
        if length > limit:
            raise ApiError(HTTPStatus.REQUEST_ENTITY_TOO_LARGE, "body too large")
        raw = self.rfile.read(length) if length else b"{}"
        try:
            data = json.loads(raw or b"{}")
        except ValueError:
            raise ApiError(HTTPStatus.BAD_REQUEST, "body must be JSON") from None
        if not isinstance(data, dict):
            raise ApiError(HTTPStatus.BAD_REQUEST, "body must be a JSON object")
        return data

    def _dispatch(self, method: str) -> None:
        try:
            self._check_host()
            url = urlparse(self.path)
            route = (method, url.path.rstrip("/") or "/")
            if route == ("GET", "/health"):
                return self._send(HTTPStatus.OK, {"ok": True, "version": VERSION})
            if route == ("GET", "/status"):
                return self._send(HTTPStatus.OK, compute_status())
            if route == ("GET", "/logs"):
                qs = parse_qs(url.query)
                source = (qs.get("source") or ["supervisor"])[0]
                if source not in LOG_SOURCES:
                    raise ApiError(HTTPStatus.BAD_REQUEST, f"source must be one of {sorted(LOG_SOURCES)}")
                try:
                    lines = max(1, min(int((qs.get("lines") or ["200"])[0]), 2000))
                except ValueError:
                    raise ApiError(HTTPStatus.BAD_REQUEST, "lines must be an integer") from None
                return self._send(HTTPStatus.OK, {"source": source, "lines": tail(LOG_DIR / LOG_SOURCES[source], lines),
                                                  "sizes": log_sizes()})
            if route == ("POST", "/logs/clear"):
                self._check_token()
                self._json_body()  # drain
                return self._send(HTTPStatus.OK, clear_logs())
            if route == ("GET", "/data"):
                return self._send(HTTPStatus.OK, inventory())
            if route == ("GET", "/datatest"):
                qs = parse_qs(url.query)
                try:
                    result = run_datatest(
                        (qs.get("symbol") or ["EURUSD"])[0],
                        (qs.get("tf") or ["M5"])[0].upper(),
                        int((qs.get("count") or ["100"])[0]),
                        float((qs.get("ticks") or ["5"])[0]),
                    )
                except ValueError as exc:
                    raise ApiError(HTTPStatus.BAD_REQUEST, str(exc)) from None
                return self._send(HTTPStatus.OK, result)
            if route == ("GET", "/bench"):
                qs = parse_qs(url.query)
                try:
                    bars = int((qs.get("bars") or ["50000"])[0])
                except ValueError:
                    raise ApiError(HTTPStatus.BAD_REQUEST, "bars must be an integer") from None
                try:
                    return self._send(HTTPStatus.OK, run_bench((qs.get("symbol") or ["EURUSD"])[0], bars))
                except ValueError as exc:
                    raise ApiError(HTTPStatus.BAD_REQUEST, str(exc)) from None
            if route == ("GET", "/strategy/supertrend"):
                qs = parse_qs(url.query)
                try:
                    period = int((qs.get("period") or ["10"])[0])
                    mult = float((qs.get("multiplier") or ["3"])[0])
                    count = int((qs.get("bars") or ["3000"])[0])
                except ValueError:
                    raise ApiError(HTTPStatus.BAD_REQUEST, "period, multiplier and bars must be numbers") from None
                return self._send(HTTPStatus.OK, run_supertrend((qs.get("symbol") or ["EURUSD"])[0], (qs.get("tf") or ["H1"])[0].upper(), period, mult, count))
            if route == ("GET", "/account/history"):
                qs = parse_qs(url.query)
                return self._send(HTTPStatus.OK, run_account_history((qs.get("range") or ["30d"])[0]))
            if route == ("GET", "/account"):
                return self._send(HTTPStatus.OK, run_account())
            if route == ("GET", "/symbols"):
                return self._send(HTTPStatus.OK, run_market_watch())
            if route == ("POST", "/data/purge"):
                self._check_token()
                return self._send(HTTPStatus.OK, purge(self._json_body()))
            if route in (("POST", "/runtime/restart-mt5"), ("POST", "/runtime/restart-bridge")):
                self._check_token()
                self._json_body()  # drain
                return self._send(HTTPStatus.ACCEPTED, request_restart(route[1].rsplit("-", 1)[1]))
            if route == ("POST", "/clipboard/get"):
                self._check_token()  # clipboard text may be private: token even for reading
                self._json_body()  # drain
                return self._send(HTTPStatus.OK, clipboard_get())
            if route == ("POST", "/clipboard/set"):
                self._check_token()
                return self._send(HTTPStatus.OK, clipboard_set(self._json_body(CLIP_BODY_MAX)))
            raise ApiError(HTTPStatus.NOT_FOUND, "not found")
        except ApiError as err:
            self._send(err.status, {"error": err.message})
        except Exception as exc:  # noqa: BLE001
            self.log_message("internal error: %r", exc)
            self._send(HTTPStatus.INTERNAL_SERVER_ERROR, {"error": "internal error"})

    def do_GET(self) -> None:  # noqa: N802
        self._dispatch("GET")

    def do_POST(self) -> None:  # noqa: N802
        self._dispatch("POST")


def make_server(host: str = LISTEN_HOST, port: int = LISTEN_PORT) -> ThreadingHTTPServer:
    server = ThreadingHTTPServer((host, port), Handler)
    server.daemon_threads = True
    return server


def main() -> None:
    threading.Thread(target=PROBE.loop, name="probe-loop", daemon=True).start()
    server = make_server()
    sys.stderr.write(f"siliconmt5 status API {VERSION} listening on {LISTEN_HOST}:{LISTEN_PORT} "
                     f"(mutations {'enabled' if API_TOKEN else 'disabled'})\n")
    server.serve_forever()


if __name__ == "__main__":
    main()
