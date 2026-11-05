"""SiliconMetaTrader5 Windows supervisor.

Launched at logon by a scheduled task (pythonw.exe, no console). Keeps three
components running: MT5 terminal, the bridge and the status API. websockify
(noVNC) is not one of them: it runs in its own task (SiliconMT5-Display,
display.py) so the screen also works while nobody is logged on. Standard
library only; imports cleanly on non-Windows hosts so the logic can be unit
tested with a FakePlatform.

Contract with smt5.ps1 (state dir = <root>\\state):
  stop.request         -> kill everything, delete the file, exit
  maintenance.request  -> stop mt5+bridge, write maintenance.ready; restart
                          when the request file disappears
  pids.json            -> {"mt5","bridge","status","supervisor"}
  state.json           -> {"phase","detail","updated","components":{...}}
  mt5cfg.sha256        -> digest of the last mt5cfg.ini written (native.sh
                          sync_mt5cfg: rewritten only when the result changes)
A stop.request older than this supervisor is stale (a stop that ended
without deleting it) and is removed at start.
No *.pid files are written (os.kill(pid, 0) would terminate on Windows).
Logs: logs\\<component>.log, kept under SMT5_LOG_MAX_MB (default 10) like
on Linux: the newest part moves to <name>.log.1.
"""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import threading
import time
from pathlib import Path

CREATE_NO_WINDOW = 0x08000000
STILL_ACTIVE = 259
STABLE_SECONDS = 60
POLL_SECONDS = 2
MAXIMIZE_WINDOW_SECONDS = 60
ORDER = ("mt5", "bridge", "status")
GROUP = ("mt5", "bridge")
DEFAULT_MT5_DIR = r"C:\Program Files\MetaTrader 5"


def log_max_bytes(env):
    """SMT5_LOG_MAX_MB (default 10, at least 1) in bytes, as on Linux."""
    v = str(env.get("SMT5_LOG_MAX_MB") or "").strip()
    mb = int(v) if v.isdigit() else 10
    return max(mb, 1) * 1024 * 1024


def rotate_file(path, max_bytes):
    """When `path` is over `max_bytes`: its newest `max_bytes` go to
    <path>.1 and it is emptied (common.sh rotate_logs). True if rotated."""
    try:
        size = os.path.getsize(path)
    except OSError:
        return False
    if size <= max_bytes:
        return False
    try:
        with open(path, "rb") as fh:
            fh.seek(size - max_bytes)
            tail = fh.read()
        tmp = path + ".1.tmp"
        with open(tmp, "wb") as fh:
            fh.write(tail)
        os.replace(tmp, path + ".1")
        with open(path, "wb"):
            pass
    except OSError:
        return False
    return True


class LogPump:
    """Copies a child's output (a pipe) into its log file and rotates the
    file at `max_bytes`. The supervisor owns the file, so rotating never
    races a child's own file handle (Windows cannot rename an open file)."""

    def __init__(self, src, path, max_bytes, chunk=65536):
        self.src, self.path, self.max, self.chunk = src, path, max_bytes, chunk

    def _open(self):
        try:
            return open(self.path, "ab")
        except OSError:
            return None

    def run(self):
        """Until the child closes its output. The pipe is always drained (a
        full pipe would block the child), even while the file is not writable."""
        rotate_file(self.path, self.max)
        fh = self._open()
        try:
            while True:
                try:
                    data = self.src.read1(self.chunk) if hasattr(self.src, "read1") else self.src.read(self.chunk)
                except (OSError, ValueError):
                    break  # the pipe broke
                if not data:
                    break
                if fh is None:
                    fh = self._open()
                    if fh is None:
                        continue
                try:
                    fh.write(data)
                    fh.flush()
                    if fh.tell() > self.max:
                        fh.close()
                        rotate_file(self.path, self.max)
                        fh = self._open()
                except OSError:
                    try:
                        fh.close()
                    except OSError:
                        pass
                    fh = None
        finally:
            if fh is not None:
                try:
                    fh.close()
                except OSError:
                    pass

    def start(self):
        t = threading.Thread(target=self.run, daemon=True)
        t.start()
        return t


def _bool(v):
    v = (v or "").strip().lower()
    return "1" if v in ("1", "true", "yes", "on") else "0" if v in ("0", "false", "no", "off") else None


def render_mt5cfg(template, env):
    """runtime/config/mt5cfg.ini with the .env overrides (native.sh
    sync_mt5cfg): MaxBars (SMT5_MAXBARS), Enabled + AllowLiveTrading
    (SMT5_ALGO_TRADING), AllowDllImport (SMT5_DLL_IMPORT)."""
    want = {}
    mb = str(env.get("SMT5_MAXBARS") or "")
    if mb.isdigit():
        want["MaxBars"] = mb
    algo = _bool(env.get("SMT5_ALGO_TRADING"))
    if algo is not None:
        want["Enabled"] = want["AllowLiveTrading"] = algo
    dll = _bool(env.get("SMT5_DLL_IMPORT"))
    if dll is not None:
        want["AllowDllImport"] = dll
    out, done = [], set()
    for line in template.splitlines():
        k = line.split("=", 1)[0].strip()
        if "=" in line and not line.lstrip().startswith(";") and k in want and k not in done:
            line = "%s=%s" % (k, want[k])
            done.add(k)
        out.append(line)
    return "\n".join(out) + "\n"


class Platform:
    """OS interface used by the Supervisor."""

    def spawn(self, argv, cwd, log, env=None):
        raise NotImplementedError

    def alive(self, pid):
        raise NotImplementedError

    def kill_tree(self, pid):
        raise NotImplementedError

    def maximize(self, pid, title_substr):
        """Maximize the top-level windows of process `pid` (MT5's main window)."""
        raise NotImplementedError

    def set_resolution(self, w, h):
        raise NotImplementedError

    def release(self, pid):
        """Forget bookkeeping for a dead/killed pid (default: nothing)."""


class WinPlatform(Platform):
    def __init__(self, log_max=10 * 1024 * 1024):
        self.log_max = log_max
        import ctypes
        from ctypes import wintypes
        self.ct = ctypes
        self.wt = wintypes
        self.k32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self.u32 = ctypes.WinDLL("user32", use_last_error=True)
        self.k32.OpenProcess.restype = wintypes.HANDLE
        self.k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        self.k32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        self.k32.CloseHandle.argtypes = [wintypes.HANDLE]
        u = self.u32
        u.IsWindowVisible.argtypes = [wintypes.HWND]
        u.IsWindowVisible.restype = wintypes.BOOL
        u.GetWindowTextLengthW.argtypes = [wintypes.HWND]
        u.GetWindowTextLengthW.restype = ctypes.c_int
        u.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        u.GetWindowTextW.restype = ctypes.c_int
        u.ShowWindow.argtypes = [wintypes.HWND, ctypes.c_int]
        u.ShowWindow.restype = wintypes.BOOL
        u.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        u.GetWindowThreadProcessId.restype = wintypes.DWORD
        u.GetWindow.argtypes = [wintypes.HWND, ctypes.c_uint]
        u.GetWindow.restype = wintypes.HWND
        u.GetClassNameW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        u.GetClassNameW.restype = ctypes.c_int
        self._enum_proc_t = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        u.EnumWindows.argtypes = [self._enum_proc_t, wintypes.LPARAM]
        u.EnumWindows.restype = wintypes.BOOL
        self._procs = {}

    def spawn(self, argv, cwd, log, env=None):
        flags = 0
        if os.path.basename(argv[0]).lower() != "terminal64.exe":
            flags = CREATE_NO_WINDOW  # MT5 needs a real window
        Path(log).parent.mkdir(parents=True, exist_ok=True)
        p = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.STDOUT, creationflags=flags)
        LogPump(p.stdout, log, self.log_max).start()
        self._procs[p.pid] = p
        return p.pid

    def alive(self, pid):
        # never os.kill(pid, 0): on Windows that terminates the process
        h = self.k32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not h:
            return False
        try:
            code = self.wt.DWORD()
            if not self.k32.GetExitCodeProcess(h, self.ct.byref(code)):
                return False
            return code.value == STILL_ACTIVE
        finally:
            self.k32.CloseHandle(h)

    def kill_tree(self, pid):
        try:
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], capture_output=True,
                           creationflags=CREATE_NO_WINDOW, timeout=15)
        except (subprocess.TimeoutExpired, OSError):
            pass  # the supervisor re-checks alive() and retries

    def release(self, pid):
        p = self._procs.pop(pid, None)
        if p is not None:
            try:
                p.wait(timeout=1)
            except Exception:  # noqa: BLE001
                pass

    def maximize(self, pid, title_substr):
        """Only visible, unowned top-level windows of MT5's own process whose
        title has `title_substr` (or whose class is MetaQuotes'): never
        another program's window, never an MT5 dialog."""
        ct, wt = self.ct, self.wt
        found = []
        proc_t = self._enum_proc_t
        u = self.u32

        def cb(hwnd, _lp):
            if not u.IsWindowVisible(hwnd) or u.GetWindow(hwnd, 4):  # GW_OWNER
                return True
            owner = wt.DWORD()
            u.GetWindowThreadProcessId(hwnd, ct.byref(owner))
            if owner.value != pid:
                return True
            n = u.GetWindowTextLengthW(hwnd)
            buf = ct.create_unicode_buffer(n + 1)
            u.GetWindowTextW(hwnd, buf, n + 1)
            cls = ct.create_unicode_buffer(256)
            u.GetClassNameW(hwnd, cls, 256)
            if title_substr in buf.value or cls.value.startswith("MetaQuotes"):
                u.ShowWindow(hwnd, 3)  # SW_MAXIMIZE
                found.append(hwnd)
            return True

        self.u32.EnumWindows(proc_t(cb), 0)
        return bool(found)

    def set_resolution(self, w, h):
        ct, wt = self.ct, self.wt

        class DEVMODEW(ct.Structure):
            _fields_ = [
                ("dmDeviceName", ct.c_wchar * 32), ("dmSpecVersion", wt.WORD),
                ("dmDriverVersion", wt.WORD), ("dmSize", wt.WORD), ("dmDriverExtra", wt.WORD),
                ("dmFields", wt.DWORD), ("dmPositionX", ct.c_long), ("dmPositionY", ct.c_long),
                ("dmDisplayOrientation", wt.DWORD), ("dmDisplayFixedOutput", wt.DWORD),
                ("dmColor", ct.c_short), ("dmDuplex", ct.c_short), ("dmYResolution", ct.c_short),
                ("dmTTOption", ct.c_short), ("dmCollate", ct.c_short),
                ("dmFormName", ct.c_wchar * 32), ("dmLogPixels", wt.WORD),
                ("dmBitsPerPel", wt.DWORD), ("dmPelsWidth", wt.DWORD), ("dmPelsHeight", wt.DWORD),
                ("dmDisplayFlags", wt.DWORD), ("dmDisplayFrequency", wt.DWORD),
                ("dmICMMethod", wt.DWORD), ("dmICMIntent", wt.DWORD), ("dmMediaType", wt.DWORD),
                ("dmDitherType", wt.DWORD), ("dmReserved1", wt.DWORD), ("dmReserved2", wt.DWORD),
                ("dmPanningWidth", wt.DWORD), ("dmPanningHeight", wt.DWORD),
            ]

        self.u32.EnumDisplaySettingsW.argtypes = [wt.LPCWSTR, wt.DWORD, ct.POINTER(DEVMODEW)]
        self.u32.EnumDisplaySettingsW.restype = wt.BOOL
        self.u32.ChangeDisplaySettingsExW.argtypes = [
            wt.LPCWSTR, ct.POINTER(DEVMODEW), wt.HWND, wt.DWORD, ct.c_void_p]
        self.u32.ChangeDisplaySettingsExW.restype = ct.c_long
        dm = DEVMODEW()
        dm.dmSize = ct.sizeof(DEVMODEW)
        if not self.u32.EnumDisplaySettingsW(None, -1, ct.byref(dm)):  # ENUM_CURRENT_SETTINGS
            return False
        dm.dmPelsWidth, dm.dmPelsHeight = w, h
        dm.dmFields = 0x80000 | 0x100000  # DM_PELSWIDTH | DM_PELSHEIGHT
        rc = self.u32.ChangeDisplaySettingsExW(None, ct.byref(dm), None, 1, None)  # CDS_UPDATEREGISTRY
        return rc == 0  # DISP_CHANGE_SUCCESSFUL


class Comp:
    def __init__(self):
        self.pid = None
        self.fails = 0
        self.started = 0.0
        self.next_start = None


def load_env(path):
    """KEY=VALUE lines; CRLF, blank lines, '#' comments and quotes tolerated."""
    out = {}
    try:
        text = Path(path).read_text(encoding="utf-8-sig")
    except OSError:
        return out
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        v = v.strip()
        if len(v) >= 2 and v[0] == v[-1] and v[0] in "\"'":
            v = v[1:-1]
        out[k.strip()] = v
    return out


class Supervisor:
    def __init__(self, root, env, platform, clock=time.monotonic, sleep=time.sleep):
        self.started_wall = time.time()
        self.root = Path(root)
        self.env = dict(env)
        self.plat = platform
        self.clock = clock
        self.sleep = sleep
        self.state_dir = self.root / "state"
        self.log_dir = self.root / "logs"
        self.runtime = self.root / "runtime"
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.log_dir.mkdir(parents=True, exist_ok=True)
        self.bridge = str(self.env.get("SMT5_BRIDGE_PORT") or 8001)
        self.status = str(self.env.get("SMT5_STATUS_PORT") or 8002)
        self.mt5_dir = self.env.get("SMT5_MT5_DIR") or DEFAULT_MT5_DIR
        self.log_max = log_max_bytes(self.env)
        self.py = str(self.root / "python" / "python.exe")
        self.comps = {n: Comp() for n in ORDER}
        self.backoff_log = {n: [] for n in ORDER}
        self.phase = "starting_terminal"
        self.detail = ""
        self.updated = time.time()
        self._written = None
        self._pids_written = None
        self.maint = False
        self.stopped = False
        self.mt5_started = None
        self.last_max = None
        self.resolution_done = False

    # -- helpers ---------------------------------------------------------
    def log(self, msg):
        path = self.log_dir / "supervisor.log"
        rotate_file(str(path), self.log_max)
        try:
            with open(path, "a", encoding="utf-8") as fh:
                fh.write("%s %s\n" % (time.strftime("%Y-%m-%d %H:%M:%S"), msg))
        except OSError:
            pass

    def _atomic(self, name, data):
        tmp = self.state_dir / (name + ".tmp")
        tmp.write_text(json.dumps(data), encoding="utf-8")
        os.replace(str(tmp), str(self.state_dir / name))

    def write_state(self, phase=None, detail=None):
        """state.json is rewritten only when phase/detail change, so `updated`
        stays the time the phase was entered (smt5_status boot timings)."""
        phase = self.phase if phase is None else phase
        detail = self.detail if detail is None else detail
        if (phase, detail) != self._written:
            self.phase, self.detail = phase, detail
            self._written = (phase, detail)
            self.updated = float(time.time())
            self._atomic("state.json", {
                "phase": phase, "detail": detail, "updated": self.updated,
                "components": {n: {"pid": c.pid} for n, c in self.comps.items()},
            })
        self.write_pids()

    def write_pids(self):
        pids = {n: c.pid for n, c in self.comps.items()}
        pids["supervisor"] = os.getpid()
        if pids != self._pids_written:
            self._pids_written = pids
            self._atomic("pids.json", pids)

    def _argv(self, name):
        if name == "mt5":
            return [str(Path(self.mt5_dir) / "terminal64.exe"), "/portable", "/config:mt5cfg.ini"]
        if name == "bridge":
            return [self.py, "-m", "siliconmetatrader5.server", "--host", "127.0.0.1", "-p", self.bridge]
        return [self.py, str(self.runtime / "status" / "smt5_status.py")]

    def sync_mt5cfg(self):
        """<MT5>/mt5cfg.ini from the template + .env; rewritten only when the
        result changes (or the file is missing), so edits survive restarts."""
        try:
            template = (self.runtime / "config" / "mt5cfg.ini").read_text(encoding="utf-8-sig")
        except OSError as e:
            self.log("mt5cfg.ini template not readable: %s" % e)
            return
        new = render_mt5cfg(template, self.env)
        digest = hashlib.sha256(new.encode("utf-8")).hexdigest()
        sha = self.state_dir / "mt5cfg.sha256"
        dest = Path(self.mt5_dir) / "mt5cfg.ini"
        try:
            old = sha.read_text(encoding="utf-8").strip()
        except OSError:
            old = ""
        if dest.exists() and digest == old:
            return
        try:
            dest.write_bytes(new.encode("utf-8"))
            sha.write_text(digest + "\n", encoding="utf-8")
        except OSError as e:
            self.log("mt5cfg.ini could not be written: %s" % e)
            return
        keys = ("MaxBars", "Enabled", "AllowLiveTrading", "AllowDllImport")
        self.log("mt5cfg.ini synchronized (%s)" % " ".join(
            l for l in new.splitlines() if l.split("=", 1)[0] in keys))

    def _child_env(self, name):
        if name == "mt5":
            return None
        env = dict(os.environ)
        env.update(self.env)
        if name == "status":
            env.update({
                "SMT5_STATUS_LISTEN": "127.0.0.1",
                "SMT5_STATUS_PORT_INTERNAL": self.status,
                "SMT5_BRIDGE_PORT_INTERNAL": self.bridge,
                "SMT5_API_TOKEN": self.env.get("SMT5_API_TOKEN", ""),
                "SMT5_MT5_DIR": self.mt5_dir,
                "SMT5_STATE_DIR": str(self.state_dir),
                "SMT5_LOG_DIR": str(self.log_dir),
                "SMT5_DISPLAY": "novnc",
                "SMT5_PLATFORM": "windows",
            })
        return env

    def _spawn(self, name, now):
        c = self.comps[name]
        cwd = self.mt5_dir if name == "mt5" else str(self.runtime)
        if name == "mt5":
            self.sync_mt5cfg()
        try:
            c.pid = self.plat.spawn(self._argv(name), cwd, str(self.log_dir / (name + ".log")),
                                    self._child_env(name))
        except Exception as e:  # noqa: BLE001 - spawn failure counts as a death
            self.log("spawn %s failed: %s" % (name, e))
            c.pid = None
            self._schedule(name, now, 0.0)
            return
        c.started = now
        c.next_start = None
        self.spawned_now = True
        if name == "mt5":
            self.mt5_started = now
            self.last_max = None
            self.write_state("starting_terminal", "")
        elif name == "bridge":
            self.write_state("starting_bridge", "")

    def _schedule(self, name, now, uptime):
        c = self.comps[name]
        if uptime >= STABLE_SECONDS:
            c.fails = 0
        delay = min(2 ** c.fails, 60)
        c.fails += 1
        self.backoff_log[name].append(delay)
        c.next_start = now + delay
        return delay

    def _kill(self, name):
        """Kill a component; True when it is gone (pid cleared)."""
        c = self.comps[name]
        if c.pid is None:
            return True
        if self.plat.alive(c.pid):  # never kill a possibly reused pid
            self.plat.kill_tree(c.pid)
            if self.plat.alive(c.pid):
                self.log("%s (pid %s) survived kill; will retry" % (name, c.pid))
                return False
        self.plat.release(c.pid)
        c.pid = None
        return True

    # -- main loop -------------------------------------------------------
    def stop(self):
        self.write_state("stopping", "")
        for name in reversed(ORDER):
            self._kill(name)
            if self.comps[name].pid is not None:
                self.plat.kill_tree(self.comps[name].pid)
        for f in ("stop.request", "maintenance.ready", "pids.json"):
            try:
                (self.state_dir / f).unlink()
            except OSError:
                pass
        self.stopped = True

    def step(self):
        if (self.state_dir / "stop.request").exists():
            self.stop()
            return
        now = self.clock()
        if not self.resolution_done:
            self.resolution_done = True
            res = (self.env.get("SMT5_RESOLUTION") or "").lower()
            if res:
                try:
                    w, h = res.split("x")
                    if not self.plat.set_resolution(int(w), int(h)):
                        self.log("resolution %s could not be applied" % res)
                except ValueError:
                    self.log("invalid SMT5_RESOLUTION %r" % res)

        self.spawned_now = False
        req = self.state_dir / "maintenance.request"
        want_maint = req.exists()
        if want_maint:
            if not self.maint:
                gone = [self._kill(n) for n in GROUP]
                for n in GROUP:
                    self.comps[n].next_start = None
                if all(gone):  # otherwise retry next step
                    self.maint = True
                    self.mt5_started = None
                    try:
                        (self.state_dir / "maintenance.ready").write_text("1", encoding="utf-8")
                    except OSError:
                        pass
            if self.maint:
                try:
                    text = req.read_text(encoding="utf-8").strip()
                except OSError:
                    text = ""
                self.write_state("maintenance", text)
        elif self.maint:
            self.maint = False
            try:
                (self.state_dir / "maintenance.ready").unlink()
            except OSError:
                pass

        # restart requests from the status API
        if not want_maint:
            r_mt5 = self.state_dir / "restart-mt5.request"
            r_bridge = self.state_dir / "restart-bridge.request"
            if r_mt5.exists():
                self._unlink(r_mt5)
                self.log("restart-mt5 requested")
                for n in GROUP:
                    self._kill(n)
                    self.comps[n].next_start = None
                self.mt5_started = None
            if r_bridge.exists():
                self._unlink(r_bridge)
                self.log("restart-bridge requested")
                self._kill("bridge")
                self.comps["bridge"].next_start = None

        # detect deaths
        for name in ORDER:
            c = self.comps[name]
            if c.pid is not None and not self.plat.alive(c.pid):
                self.log("%s (pid %s) exited" % (name, c.pid))
                self.plat.release(c.pid)
                c.pid = None
                self._schedule(name, now, now - c.started)
                if name in GROUP:
                    self.mt5_started = None
                    for other in GROUP:
                        if other != name:
                            self._kill(other)
                            oc = self.comps[other]
                            oc.fails = c.fails
                            oc.next_start = c.next_start
            elif c.pid is not None and c.fails and now - c.started >= STABLE_SECONDS:
                c.fails = 0

        # (re)spawn in order
        for name in ORDER:
            c = self.comps[name]
            if c.pid is not None:
                continue
            if (want_maint or self.maint) and name in GROUP:
                continue
            if c.next_start is not None and now < c.next_start:
                continue
            self._spawn(name, now)

        # window handling
        if self.mt5_started is not None and now - self.mt5_started <= MAXIMIZE_WINDOW_SECONDS:
            if self.last_max is None or now - self.last_max >= POLL_SECONDS:
                self.last_max = now
                self.plat.maximize(self.comps["mt5"].pid, "MetaTrader 5")

        if not self.maint:
            if any(self.comps[n].pid is None for n in GROUP):
                self.write_state("error", "restarting MT5/bridge after exit")
            elif self.spawned_now:
                self.write_pids()  # phase already starting_terminal/starting_bridge
            elif all(self.comps[n].pid is not None for n in ORDER):
                self.write_state("running", "")
            else:
                self.write_pids()
        else:
            self.write_pids()

    @staticmethod
    def _unlink(path):
        try:
            path.unlink()
        except OSError:
            pass

    def clear_stale_stop(self):
        """A stop.request from before this supervisor started was meant for
        an earlier one (or a stop that failed half way): not for us."""
        f = self.state_dir / "stop.request"
        try:
            if f.stat().st_mtime < self.started_wall:
                f.unlink()
                self.log("stale stop.request removed")
        except OSError:
            pass

    def run(self):
        self.clear_stale_stop()
        while not self.stopped:
            try:
                self.step()
            except Exception as e:  # noqa: BLE001 - supervisor must not die
                self.log("step error: %r" % (e,))
            if self.stopped:
                break
            self.sleep(POLL_SECONDS)


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=str(Path(__file__).resolve().parents[2]))
    args = ap.parse_args(argv)
    root = Path(args.root)
    env = load_env(root / "runtime" / ".env")
    env.update({k: v for k, v in os.environ.items() if k.startswith("SMT5_")})
    env.setdefault("SMT5_MT5_DIR", DEFAULT_MT5_DIR)
    Supervisor(root, env, WinPlatform(log_max_bytes(env))).run()


if __name__ == "__main__":
    main()
