"""Tests for smt5_status.py against a fake MT5 data folder.

Run: python3 -m unittest runtime/status/test_smt5_status.py -v
"""
import importlib
import json
import os
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOKEN = "test-token-123"


def write(path: Path, size: int) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"x" * size)


class StatusApiTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        root = Path(cls.tmp.name)
        cls.mt5 = root / "MetaTrader 5"
        cls.state = root / "state"
        cls.logs = root / "logs"
        cls.state.mkdir()
        cls.logs.mkdir()
        os.environ.update({
            "SMT5_MT5_DIR": str(cls.mt5),
            "SMT5_STATE_DIR": str(cls.state),
            "SMT5_LOG_DIR": str(cls.logs),
            "SMT5_API_TOKEN": TOKEN,
            "SMT5_BRIDGE_PORT_INTERNAL": "1",  # nothing listens there
            "SMT5_MAINTENANCE_TIMEOUT": "5",
        })
        sys.path.insert(0, str(HERE))
        import smt5_status
        cls.m = importlib.reload(smt5_status)
        cls.server = cls.m.make_server("127.0.0.1", 0)
        cls.port = cls.server.server_address[1]
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.tmp.cleanup()

    def setUp(self):
        # Fresh fake data tree for every test.
        import shutil
        shutil.rmtree(self.mt5, ignore_errors=True)
        h = self.mt5 / "bases" / "Demo-Server" / "history"
        t = self.mt5 / "bases" / "Demo-Server" / "ticks"
        for year in range(2019, 2027):
            write(h / "EURUSD" / f"{year}.hcc", 1000)
        for year in (2024, 2025):
            write(h / "XAUUSD" / f"{year}.hcc", 500)
        write(h / "EURUSD" / "cache" / "H1.hc", 300)
        write(t / "EURUSD" / "202607.tkc", 2000)
        write(t / "EURUSD" / "202608.tkc", 2000)
        write(t / "EURUSD" / "202609.tkc", 2000)
        write(t / "EURUSD" / "ticks.dat", 1500)          # live tick cache (MT5 build 6295)
        write(t / "GBPUSD" / "ticks.dat", 1500)
        write(self.mt5 / "Tester" / "Agent-127.0.0.1-3000" / "bases" / "x.dat", 700)
        write(self.mt5 / "Tester" / "settings.ini", 10)
        write(self.mt5 / "terminal64.exe", 1)
        (self.mt5 / "mt5cfg.ini").write_text("[Charts]\nMaxBars=100000\n")
        outside = Path(self.tmp.name) / "outside.hcc"
        write(outside, 5)
        link = h / "EURUSD" / "2018.hcc"
        if not link.exists():
            os.symlink(outside, link)
        self.outside = outside

    # ---- helpers -----------------------------------------------------
    def call(self, method, path, body=None, token=TOKEN, host=None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method=method)
        if token:
            req.add_header("Authorization", f"Bearer {token}")
        if data is not None:
            req.add_header("Content-Type", "application/json")
        if host:
            req.add_header("Host", host)
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as err:
            return err.code, json.loads(err.read())

    def fake_supervisor(self):
        """Acknowledge one maintenance request like supervisor.sh does."""
        req = self.state / "maintenance.request"
        ack = self.state / "maintenance.ready"

        def run():
            deadline = time.time() + 5
            while not req.exists() and time.time() < deadline:
                time.sleep(0.05)
            ack.touch()
            while req.exists() and time.time() < deadline:
                time.sleep(0.05)
            ack.unlink(missing_ok=True)

        th = threading.Thread(target=run, daemon=True)
        th.start()
        return th

    # ---- tests -------------------------------------------------------
    def test_health_and_status(self):
        code, body = self.call("GET", "/health", token=None)
        self.assertEqual(code, 200)
        self.assertTrue(body["ok"])
        (self.state / "state.json").write_text(json.dumps({"phase": "running"}))
        code, body = self.call("GET", "/status", token=None)
        self.assertEqual(code, 200)
        self.assertEqual(body["mt5"]["maxbars"], 100000)
        self.assertFalse(body["bridge"]["listening"])
        self.assertIn(body["phase"], {"starting_terminal", "starting_bridge"})
        (self.state / "state.json").write_text(json.dumps({"phase": "installing_mt5"}))
        self.assertEqual(self.call("GET", "/status")[1]["phase"], "installing_mt5")

    def test_low_disk_overrides_phase(self):
        (self.state / "state.json").write_text(json.dumps({"phase": "preparing_prefix", "detail": "copying"}))
        old = self.m.MIN_FREE_MB
        self.m.MIN_FREE_MB = 10 ** 12          # pretend the disk is (almost) full
        try:
            body = self.call("GET", "/status")[1]
        finally:
            self.m.MIN_FREE_MB = old
        self.assertEqual(body["phase"], "error")
        self.assertIn("disk almost full", body["detail"])
        self.assertEqual(self.call("GET", "/status")[1]["detail"], "copying")

    def test_host_header_rejected(self):
        code, _ = self.call("GET", "/status", host="evil.example:8002")
        self.assertEqual(code, 403)

    def test_inventory(self):
        code, inv = self.call("GET", "/data", token=None)
        self.assertEqual(code, 200)
        server = inv["servers"][0]
        self.assertEqual(server["name"], "Demo-Server")
        eur = next(s for s in server["history"] if s["symbol"] == "EURUSD")
        self.assertEqual(len(eur["years"]), 8)            # symlink 2018.hcc ignored
        self.assertEqual(eur["cache_bytes"], 300)
        eur_ticks = next(s for s in server["ticks"] if s["symbol"] == "EURUSD")
        self.assertEqual(eur_ticks["months"]["202608"], 2000)
        self.assertEqual(eur_ticks["other_bytes"], 1500)
        gbp_ticks = next(s for s in server["ticks"] if s["symbol"] == "GBPUSD")
        self.assertEqual((gbp_ticks["months"], gbp_ticks["other_bytes"]), ({}, 1500))
        self.assertEqual(inv["tester_bytes"], 710)

    def test_appdata_data_folder_is_found(self):
        """Under Wine MT5 may keep bases/ in AppData instead of the install dir."""
        import shutil
        bases = self.mt5 / "bases"
        term = Path(self.tmp.name) / "wine" / "drive_c" / "users" / "root" / "AppData" / "Roaming" / "MetaQuotes" / "Terminal" / "D0E8209F77C8CF37AD8BF550E51FF075"
        term.mkdir(parents=True, exist_ok=True)
        (term / "origin.txt").write_bytes("C:\\Program Files\\MetaTrader 5".encode("utf-16"))
        shutil.move(str(bases), str(term / "bases"))
        old = self.m.WINEPREFIX
        self.m.WINEPREFIX = Path(self.tmp.name) / "wine"
        try:
            inv = self.call("GET", "/data")[1]
            self.assertEqual(inv["root"], str(term))
            self.assertEqual(inv["servers"][0]["name"], "Demo-Server")
            body = self.call("POST", "/data/purge", {"items": [{"kind": "history", "symbol": "EURUSD", "years": [2019]}]})[1]
            self.assertEqual(body["sample"], ["bases/Demo-Server/history/EURUSD/2019.hcc"])
        finally:
            self.m.WINEPREFIX = old
            shutil.rmtree(Path(self.tmp.name) / "wine", ignore_errors=True)

    def test_case_insensitive_folders(self):
        """Wine may create Bases/History/Ticks with capitals on a case-sensitive fs."""
        import os as _os
        b = self.mt5 / "bases"
        _os.rename(b / "Demo-Server" / "history", b / "Demo-Server" / "History")
        _os.rename(b, self.mt5 / "Bases")
        inv = self.call("GET", "/data")[1]
        self.assertEqual(inv["root"], str(self.mt5))
        eur = next(s for s in inv["servers"][0]["history"] if s["symbol"] == "EURUSD")
        self.assertEqual(len(eur["years"]), 8)
        body = self.call("POST", "/data/purge", {"items": [{"kind": "history", "symbol": "EURUSD", "years": [2020]}]})[1]
        self.assertEqual(body["files"], 1)

    def test_purge_requires_token(self):
        spec = {"items": [{"kind": "cache"}]}
        self.assertEqual(self.call("POST", "/data/purge", spec, token=None)[0], 401)
        self.assertEqual(self.call("POST", "/data/purge", spec, token="wrong")[0], 401)

    def test_dry_run_is_default_and_deletes_nothing(self):
        spec = {"items": [{"kind": "history", "symbol": "EURUSD", "before_year": 2023}]}
        code, body = self.call("POST", "/data/purge", spec)
        self.assertEqual(code, 200)
        self.assertTrue(body["dry_run"])
        self.assertEqual(body["files"], 4)                 # 2019..2022
        self.assertEqual(body["bytes"], 4000)
        self.assertTrue((self.mt5 / "bases/Demo-Server/history/EURUSD/2019.hcc").exists())

    def test_selectors(self):
        def files(item):
            return self.call("POST", "/data/purge", {"items": [item]})[1]["files"]
        self.assertEqual(files({"kind": "history", "symbol": "*", "years": [2024]}), 2)
        self.assertEqual(files({"kind": "history", "server": "Demo-Server", "symbol": "XAUUSD", "all": True}), 2)
        self.assertEqual(files({"kind": "ticks", "symbol": "EURUSD", "before_month": 202609}), 2)
        self.assertEqual(files({"kind": "ticks", "months": ["202607"]}), 1)
        self.assertEqual(files({"kind": "ticks", "symbol": "*", "all": True}), 5)   # 3 .tkc + 2 ticks.dat
        self.assertEqual(files({"kind": "cache"}), 1)
        self.assertEqual(files({"kind": "tester"}), 1)     # settings.ini is kept

    def test_bad_requests(self):
        bad = [
            {"items": []},
            {"items": [{"kind": "history", "symbol": "../../etc", "all": True}]},
            {"items": [{"kind": "history", "symbol": "EURUSD"}]},                       # no selector
            {"items": [{"kind": "history", "years": [2020], "before_year": 2021}]},     # two selectors
            {"items": [{"kind": "history", "years": ["20x0"]}]},
            {"items": [{"kind": "history", "all": "yes"}]},
            {"items": [{"kind": "rm -rf"}]},
            {"items": [{"kind": "cache"}], "dry_run": "false"},
        ]
        for spec in bad:
            with self.subTest(spec=spec):
                self.assertEqual(self.call("POST", "/data/purge", spec)[0], 400)

    def test_real_purge_uses_maintenance_handshake(self):
        sup = self.fake_supervisor()
        spec = {"dry_run": False, "items": [
            {"kind": "history", "symbol": "EURUSD", "before_year": 2023},
            {"kind": "cache", "symbol": "EURUSD"},
        ]}
        code, body = self.call("POST", "/data/purge", spec)
        sup.join(5)
        self.assertEqual(code, 200, body)
        self.assertTrue(body["deleted"])
        self.assertEqual(body["removed"], 5)
        self.assertEqual(body["freed_bytes"], 4300)
        hist = self.mt5 / "bases/Demo-Server/history/EURUSD"
        self.assertFalse((hist / "2022.hcc").exists())
        self.assertTrue((hist / "2023.hcc").exists())
        self.assertFalse((hist / "cache").exists())        # empty cache dir pruned
        self.assertTrue(self.outside.exists())              # symlink target untouched
        self.assertFalse((self.state / "maintenance.request").exists())

    def test_server_disappears_after_full_purge(self):
        (self.mt5 / "bases" / "Demo-Server" / "ticks" / "EMPTY").mkdir(parents=True)
        sup = self.fake_supervisor()
        spec = {"dry_run": False, "items": [
            {"kind": "history", "server": "Demo-Server", "symbol": "*", "all": True},
            {"kind": "cache", "server": "Demo-Server", "symbol": "*"},
            {"kind": "ticks", "server": "Demo-Server", "symbol": "*", "all": True},
        ]}
        code, body = self.call("POST", "/data/purge", spec)
        sup.join(5)
        self.assertEqual(code, 200, body)
        self.assertEqual(self.call("GET", "/data")[1]["servers"], [])

    def test_real_purge_times_out_without_supervisor(self):
        spec = {"dry_run": False, "items": [{"kind": "cache"}]}
        code, body = self.call("POST", "/data/purge", spec)
        self.assertEqual(code, 503)
        self.assertTrue((self.mt5 / "bases/Demo-Server/history/EURUSD/cache/H1.hc").exists())
        self.assertFalse((self.state / "maintenance.request").exists())

    def test_log_sizes_and_clear(self):
        (self.logs / "bridge.log").write_text("x" * 100)
        (self.logs / "bridge.log.1").write_text("y" * 50)
        code, body = self.call("GET", "/logs?source=bridge&lines=5")
        self.assertEqual(code, 200)
        self.assertEqual(body["sizes"]["bridge"], 150)
        self.assertEqual(self.call("POST", "/logs/clear", {}, token=None)[0], 401)
        code, body = self.call("POST", "/logs/clear", {}, token=TOKEN)
        self.assertEqual(code, 200)
        self.assertGreaterEqual(body["freed_bytes"], 150)
        self.assertEqual((self.logs / "bridge.log").stat().st_size, 0)

    def test_restart_and_logs(self):
        code, body = self.call("POST", "/runtime/restart-mt5", {})
        self.assertEqual(code, 202)
        self.assertTrue((self.state / "restart-mt5.request").exists())
        (self.logs / "mt5.log").write_text("\n".join(f"line {i}" for i in range(500)))
        code, body = self.call("GET", "/logs?source=mt5&lines=3", token=None)
        self.assertEqual(body["lines"], ["line 497", "line 498", "line 499"])
        self.assertEqual(self.call("GET", "/logs?source=../../etc/passwd")[0], 400)
        self.assertEqual(self.call("GET", "/nope")[0], 404)


class NativeModeTest(unittest.TestCase):
    """macOS native mode: pid files instead of /proc, env overrides."""

    def setUp(self):
        sys.path.insert(0, str(HERE))
        import smt5_status
        self.m = smt5_status
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def test_pid_file(self):
        f = Path(self.tmp.name) / "mt5.pid"
        self.assertIsNone(self.m.pid_alive(f))
        f.write_text(str(os.getpid()))
        self.assertTrue(self.m.pid_alive(f))
        f.write_text("999999999")
        self.assertFalse(self.m.pid_alive(f))
        f.write_text("garbage")
        self.assertIsNone(self.m.pid_alive(f))

    def test_env_overrides(self):
        os.environ["SMT5_CPU_TRANSLATION"] = "rosetta2"
        os.environ["SMT5_STARTED_AT"] = "1700000000"
        try:
            self.assertEqual(self.m.cpu_translation(), "rosetta2")
            self.assertEqual(self.m.boot_timings()["container_started"], 1700000000.0)
        finally:
            del os.environ["SMT5_CPU_TRANSLATION"], os.environ["SMT5_STARTED_AT"]
        self.assertNotEqual(self.m.cpu_translation(), "rosetta2")


class WindowsModeTest(unittest.TestCase):
    """Windows server mode: PowerShell process list, portable data dir, no os.kill."""

    def setUp(self):
        sys.path.insert(0, str(HERE))
        import smt5_status
        from unittest import mock
        self.m = smt5_status
        self.mock = mock
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        p = mock.patch.object(smt5_status, "IS_WINDOWS", True)
        p.start()
        self.addCleanup(p.stop)

    def _fake_run(self, stdout):
        calls = []

        def run(cmd, *a, **kw):
            calls.append((cmd, kw))
            return self.mock.Mock(stdout=stdout, returncode=0)
        return run, calls

    def test_windows_process_list_parsed(self):
        out = (b'C:\\Windows\\system32\\svchost.exe -k x\r\n'
               b'"C:\\Program Files\\MetaTrader 5\\terminal64.exe" /portable\r\n')
        run, calls = self._fake_run(out)
        with self.mock.patch("subprocess.run", run):
            self.assertTrue(self.m.process_running("terminal64.exe"))
            self.assertFalse(self.m.process_running("nothing-like-this"))
        cmd, kw = calls[0]
        self.assertEqual(cmd[0].lower(), "powershell")
        self.assertIn("-NoProfile", cmd)
        self.assertIn("Get-CimInstance Win32_Process", " ".join(cmd))
        self.assertEqual(kw.get("timeout"), 5)
        import subprocess
        self.assertEqual(kw.get("creationflags"), getattr(subprocess, "CREATE_NO_WINDOW", 0x08000000))
        self.assertEqual(kw.get("stdin"), subprocess.DEVNULL)

    def test_windows_process_list_failure_is_false(self):
        import subprocess
        with self.mock.patch("subprocess.run", side_effect=subprocess.TimeoutExpired("powershell", 5)):
            self.assertFalse(self.m.process_running("terminal64.exe"))

    def test_windows_terminal_dir_is_mt5_dir(self):
        old = os.environ.get("SMT5_MT5_DATA_DIR")
        os.environ.pop("SMT5_MT5_DATA_DIR", None)
        try:
            # no bases/ yet: still MT5_DIR, WINEPREFIX scan skipped
            with self.mock.patch.object(self.m, "WINEPREFIX", Path(self.tmp.name) / "wine"):
                wp = Path(self.tmp.name) / "wine" / "drive_c" / "users" / "u" / "AppData" / "Roaming" / "MetaQuotes" / "Terminal" / "ABC"
                (wp / "bases").mkdir(parents=True)
                self.assertEqual(self.m.mt5_data_dir(), self.m.MT5_DIR)
        finally:
            if old is not None:
                os.environ["SMT5_MT5_DATA_DIR"] = old

    def test_windows_cpu_translation_none(self):
        old = os.environ.pop("SMT5_CPU_TRANSLATION", None)
        try:
            self.assertEqual(self.m.cpu_translation(), "none")
        finally:
            if old is not None:
                os.environ["SMT5_CPU_TRANSLATION"] = old

    def test_windows_pid_liveness_never_calls_os_kill(self):
        pids = Path(self.tmp.name) / "pids.json"
        pids.write_text(json.dumps({"mt5": 4242, "bridge": 1}))

        class K32:
            def OpenProcess(self, access, inherit, pid):
                self.opened = (access, pid)
                return 77 if pid == 4242 else 0

            def GetExitCodeProcess(self, handle, ref):
                ref._obj.value = 259  # STILL_ACTIVE
                return 1

            def CloseHandle(self, handle):
                self.closed = handle
                return 1
        k32 = K32()
        with self.mock.patch("os.kill", side_effect=AssertionError("os.kill on Windows")), \
                self.mock.patch.object(self.m, "STATE_DIR", Path(self.tmp.name)), \
                self.mock.patch.object(self.m, "_kernel32", return_value=k32):
            self.assertTrue(self.m.pid_alive(pids))
            self.assertEqual(k32.opened, (0x1000, 4242))
            self.assertEqual(k32.closed, 77)
            self.assertTrue(self.m.terminal_running())
            pids.write_text(json.dumps({"mt5": 5}))
            self.assertFalse(self.m.pid_alive(pids))  # OpenProcess -> 0
            self.assertIsNone(self.m.pid_alive(Path(self.tmp.name) / "missing.json"))

    def test_windows_exited_pid_not_alive(self):
        class K32:
            def OpenProcess(self, a, i, pid):
                return 9

            def GetExitCodeProcess(self, h, ref):
                ref._obj.value = 0
                return 1

            def CloseHandle(self, h):
                return 1
        with self.mock.patch("os.kill", side_effect=AssertionError("os.kill")), \
                self.mock.patch.object(self.m, "_kernel32", return_value=K32()):
            self.assertFalse(self.m._win_pid_alive(123))

    def test_windows_kernel32_prototypes(self):
        import ctypes
        fake = self.mock.Mock()
        with self.mock.patch("ctypes.WinDLL", return_value=fake, create=True) as w, \
                self.mock.patch.object(self.m, "_KERNEL32", None):
            k = self.m._kernel32()
            self.assertIs(k, fake)
            self.assertIs(self.m._kernel32(), fake)  # cached
            w.assert_called_once_with("kernel32", use_last_error=True)
        self.assertIs(fake.GetTickCount64.restype, ctypes.c_ulonglong)
        self.assertEqual(fake.GetTickCount64.argtypes, [])
        self.assertEqual(len(fake.OpenProcess.argtypes), 3)
        self.assertEqual(len(fake.GetExitCodeProcess.argtypes), 2)

    def test_windows_ctypes_failure_falls_back_to_process_list(self):
        with self.mock.patch.object(self.m, "STATE_DIR", Path(self.tmp.name)), \
                self.mock.patch("os.kill", side_effect=AssertionError("os.kill")), \
                self.mock.patch.object(self.m, "_kernel32", side_effect=OSError("no kernel32")), \
                self.mock.patch.object(self.m, "process_running", return_value=True):
            (Path(self.tmp.name) / "pids.json").write_text('{"mt5": 4242}')
            self.assertIsNone(self.m._win_pid_alive(4242))
            self.assertTrue(self.m.terminal_running())

    def test_windows_terminal_running_falls_back_to_process_list(self):
        with self.mock.patch.object(self.m, "STATE_DIR", Path(self.tmp.name)), \
                self.mock.patch("os.kill", side_effect=AssertionError("os.kill")), \
                self.mock.patch.object(self.m, "process_running", return_value=True) as pr:
            self.assertTrue(self.m.terminal_running())  # no pids.json
            pr.assert_called_with("terminal64.exe")

    def test_windows_disk_usage_uses_shutil(self):
        import collections
        du = collections.namedtuple("du", "total used free")
        with self.mock.patch("shutil.disk_usage", return_value=du(100 * 1048576, 40 * 1048576, 60 * 1048576)) as s, \
                self.mock.patch("os.statvfs", side_effect=AssertionError("statvfs"), create=True):
            r = self.m.disk_usage(Path(self.tmp.name) / "state")
        self.assertEqual(r["free_mb"], 60)
        self.assertEqual(r["total_mb"], 100)
        self.assertEqual(set(r), {"path", "free_mb", "total_mb"})
        self.assertTrue(s.called)

    def test_windows_boot_time_uses_gettickcount64(self):
        k32 = self.mock.Mock()
        k32.GetTickCount64.return_value = 3_600_000
        env = {k: v for k, v in os.environ.items() if k != "SMT5_STARTED_AT"}
        with self.mock.patch.dict(os.environ, env, clear=True), \
                self.mock.patch.object(self.m, "_kernel32", return_value=k32), \
                self.mock.patch.object(self.m.time, "time", return_value=1_000_000.0):
            self.assertEqual(self.m.boot_timings()["container_started"], 996400.0)


class MarketWatchTest(unittest.TestCase):
    def test_market_watch_through_fake_ops(self):
        sys.path.insert(0, str(HERE))
        import smt5_status

        class Ops:
            def market_watch(self):
                return ["EURUSD", "US500"], 900

            def close(self):
                pass

        orig = smt5_status.port_open
        smt5_status.port_open = lambda port: True
        try:
            self.assertEqual(smt5_status.run_market_watch(lambda: Ops()), {"symbols": ["EURUSD", "US500"], "total": 900})
        finally:
            smt5_status.port_open = orig

    def test_bridge_closed(self):
        sys.path.insert(0, str(HERE))
        import smt5_status
        orig = smt5_status.port_open
        smt5_status.port_open = lambda port: False
        try:
            with self.assertRaises(smt5_status.ApiError):
                smt5_status.run_market_watch(lambda: None)
        finally:
            smt5_status.port_open = orig


class BenchTest(unittest.TestCase):
    def test_bench_endpoint_logic(self):
        sys.path.insert(0, str(HERE))
        import smt5_status

        class Ops:
            def initialize(self):
                return True, ""
            def select(self, s):
                return True
            def cpu_seconds(self, n):
                return 2.0
            def rates_timed(self, s, n):
                return n, 0.4
            def rates_from_pos(self, s, tf, n):
                return [(0, 1.0, 1.0, 1.0, 1.0, 1)] * n
            def tick_roundtrip(self, s):
                pass
            def close(self):
                pass

        orig = smt5_status.port_open
        smt5_status.port_open = lambda port: True
        try:
            r = smt5_status.run_bench("EURUSD", 50_000, lambda: Ops())
            self.assertTrue(r["ok"], r)
            self.assertEqual(r["rates_rows"], 50_000)
            self.assertIn("boot_s", r)
            self.assertEqual(r["wine_flavor"], os.environ.get("SMT5_WINE_FLAVOR", "winehq"))
            with self.assertRaises(smt5_status.ApiError):
                smt5_status.run_bench("EURUSD", 10, lambda: Ops())
        finally:
            smt5_status.port_open = orig


class StrategyTest(unittest.TestCase):
    def test_supertrend_endpoint_logic(self):
        sys.path.insert(0, str(HERE))
        import smt5_status
        from test_supertrend_strategy import make_bars

        bars = make_bars(800)

        class Ops:
            def initialize(self):
                return True, ""
            def select(self, s):
                return s != "NOPE"
            def symbol_info(self, s):
                return 5, 0.00001
            def rates_from_pos(self, s, tf, n):
                return [(b.time, b.open, b.high, b.low, b.close, b.volume) for b in bars[-n:]]
            def close(self):
                pass

        orig = smt5_status.port_open
        smt5_status.port_open = lambda port: True
        try:
            r = smt5_status.run_supertrend("EURUSD", "H1", 10, 3.0, 500, lambda: Ops())
            self.assertEqual(r["count"], 500)                      # 501 rows, forming one dropped
            self.assertEqual(r["forming"], bars[-1].time)
            self.assertEqual(r["last_closed"], bars[-2].time)
            self.assertEqual(len(r["chart"]["bars"]), smt5_status.STRATEGY_CHART_BARS)
            self.assertTrue(all(s["time"] <= bars[-2].time for s in r["signals"]))
            self.assertIn(r["trend"], (1, -1))
            json.dumps(r)
            self.assertEqual(r["requested"], 500)
            # MT5 answers empty at first (building D1), then has only 300 bars
            smt5_status.STRATEGY_RETRY_WAIT = 0

            class Short(Ops):
                calls = 0
                def rates_from_pos(self, s, tf, n):
                    Short.calls += 1
                    if Short.calls == 1 or n > 301:
                        return []
                    return super().rates_from_pos(s, tf, n)
            r = smt5_status.run_supertrend("EURUSD", "D1", 10, 3.0, 3000, lambda: Short())
            self.assertGreater(r["count"], 100)
            self.assertLess(r["count"], 3000)
            for bad in [("EURUSD", "H2", 10, 3.0, 500), ("EURUSD", "H1", 0, 3.0, 500), ("EURUSD", "H1", 10, 3.0, 50)]:
                with self.assertRaises(smt5_status.ApiError):
                    smt5_status.run_supertrend(*bad, lambda: Ops())
            with self.assertRaises(smt5_status.ApiError) as cm:
                smt5_status.run_supertrend("NOPE", "H1", 10, 3.0, 500, lambda: Ops())
            self.assertIn("not available", str(cm.exception))
        finally:
            smt5_status.port_open = orig


class AccountTest(unittest.TestCase):
    def test_account_and_login(self):
        sys.path.insert(0, str(HERE))
        import smt5_status

        class AI:
            def __init__(self, login, server):
                self.login, self.server, self.name, self.company = login, server, "Demo", "Broker Ltd"
                self.currency, self.leverage, self.trade_mode = "USD", 100, 0

        class Client:
            current = AI(111, "A-Demo")
            def initialize(self):
                return True
            def terminal_info(self):
                return type("T", (), {"connected": True})()
            def account_info(self):
                return Client.current
            def last_error(self):
                return (-6, "Authorization failed")

        class Ops:
            c = Client()
            def close(self):
                pass

        orig = smt5_status.port_open
        smt5_status.port_open = lambda port: True
        try:
            r = smt5_status.run_account(lambda: Ops())
            self.assertEqual(r["account"]["login"], 111)
            self.assertEqual(r["account"]["trade_mode"], "demo")
        finally:
            smt5_status.port_open = orig


class HistoryTest(unittest.TestCase):
    def test_summarize_history(self):
        sys.path.insert(0, str(HERE))
        import smt5_status as m
        D = 86400
        deals = [
            (1 * D, 2, 0, 25000.0, 0, 0, 0, "", 0, 0),          # deposit
            (2 * D, 0, 0, 0.0, -3.0, 0, 0, "EURUSD", 1, 7),     # open buy, commission
            (2 * D + 60, 1, 1, 120.0, -3.0, -1.0, 0, "EURUSD", 1, 7),  # close: +120
            (3 * D, 1, 0, 0.0, 0, 0, 0, "XAUUSD", 1, 8),
            (3 * D + 60, 0, 1, -50.0, 0, 0, 0, "XAUUSD", 1, 8),  # close: -50
            (4 * D, 3, 0, 500.0, 0, 0, 0, "", 0, 0),            # credit: not balance
            (5 * D, 2, 0, -1000.0, 0, 0, 0, "", 0, 0),          # withdrawal
        ]
        balance = 25000 - 3 + 120 - 3 - 1 - 50 - 1000
        r = m.summarize_history(deals, balance, balance + 12.5, 0, 6 * D)
        self.assertEqual(r["start_balance"], 0)
        self.assertEqual(r["trading_pl"], 63.0)
        self.assertEqual(r["deposits"], 25000.0)
        self.assertEqual(r["withdrawals"], -1000.0)
        self.assertEqual((r["trades"], r["wins"], r["losses"]), (2, 1, 1))
        self.assertEqual(r["floating"], 12.5)
        self.assertEqual(r["curve"][-1][1], balance)
        self.assertEqual(dict(r["by_symbol"]), {"EURUSD": 113.0, "XAUUSD": -50.0})
        # a window after the deposit: the start balance comes from the account's balance
        r2 = m.summarize_history(deals, balance, balance, 2 * D, 6 * D)
        self.assertEqual(r2["start_balance"], 25000.0)
        self.assertEqual(r2["deposits"], 0)
        self.assertEqual(r2["trading_pl"], 63.0)


class ProbeQueryTest(unittest.TestCase):
    """BridgeProbe._query against a fake `siliconmetatrader5` client."""

    def test_account_numbers_and_counts(self):
        import types
        from collections import namedtuple

        Ti = namedtuple("Ti", "connected build trade_allowed")
        Ai = namedtuple("Ai", "login server company currency trade_mode leverage balance equity profit margin margin_free margin_level")

        class Fake:
            def __init__(self, host, port):
                pass
            def ping(self):
                return True
            def initialize(self):
                return True
            def terminal_info(self):
                return Ti(True, 6295, True)
            def account_info(self):
                return Ai(1520001234, "FTMO-Demo", "FTMO", "USD", 0, 100, 10000.5, 10012.25, 11.75, 50.0, 9962.25, 20024.5)
            def positions_total(self):
                return 2
            def orders_total(self):
                raise RuntimeError("not supported")
            def close(self):
                pass

        fake = types.ModuleType("siliconmetatrader5")
        fake.MetaTrader5 = Fake
        sys.modules["siliconmetatrader5"] = fake
        try:
            sys.path.insert(0, str(HERE))
            import smt5_status
            r = smt5_status.BridgeProbe._query()
        finally:
            del sys.modules["siliconmetatrader5"]
        self.assertTrue(r["ok"])
        self.assertEqual(r["account"]["login_masked"], "•••234")
        self.assertEqual(r["account"]["equity"], 10012.25)
        self.assertEqual(r["account"]["leverage"], 100)
        self.assertEqual(r["positions"], 2)
        self.assertIsNone(r["orders"])
        json.dumps(r)



class ClipboardTest(unittest.TestCase):
    """Server mode clipboard sync: /clipboard/get and /clipboard/set."""

    def setUp(self):
        sys.path.insert(0, str(HERE))
        import smt5_status
        from unittest import mock
        self.m = smt5_status
        self.mock = mock

    def test_routes_need_the_token(self):
        api = StatusApiTest
        api.setUpClass()
        self.addCleanup(lambda: api.server.server_close())
        self.addCleanup(api.tearDownClass)
        t = api("call")
        with self.mock.patch.object(api.m, "clipboard_get", return_value={"text": "x"}):
            self.assertEqual(t.call("POST", "/clipboard/get", {}, token=None)[0], 401)
            self.assertEqual(t.call("POST", "/clipboard/get", {}), (200, {"text": "x"}))
        with self.mock.patch.object(api.m, "clipboard_set", return_value={"ok": True, "chars": 1}) as cs:
            self.assertEqual(t.call("POST", "/clipboard/set", {"text": "y"}, token=None)[0], 401)
            self.assertEqual(t.call("POST", "/clipboard/set", {"text": "y"})[0], 200)
            cs.assert_called_once_with({"text": "y"})
        # a body over the normal 64 KiB limit is accepted for the clipboard
        big = "a" * (100 * 1024)
        with self.mock.patch.object(api.m, "clipboard_set", return_value={"ok": True, "chars": len(big)}):
            self.assertEqual(t.call("POST", "/clipboard/set", {"text": big})[0], 200)

    def test_linux_uses_xclip_on_the_display(self):
        calls = []

        def run(argv, input=None, capture_output=True, timeout=None, env=None):
            calls.append((argv, input, env.get("DISPLAY")))
            out = "şimdi €".encode("utf-8") if "-o" in argv else b""
            return self.mock.Mock(returncode=0, stdout=out, stderr=b"")

        with self.mock.patch.object(self.m, "IS_WINDOWS", False), self.mock.patch("subprocess.run", side_effect=run), \
                self.mock.patch.dict(os.environ, {"DISPLAY": ":7"}):
            self.assertEqual(self.m.clipboard_get(), {"text": "şimdi €"})
            self.assertEqual(self.m.clipboard_set({"text": "çok"}), {"ok": True, "chars": 3})
        self.assertEqual(calls[0][0][:3], ["xclip", "-selection", "clipboard"])
        self.assertEqual(calls[0][2], ":7")
        self.assertEqual(calls[-1][1], "çok".encode("utf-8"))

    def test_linux_without_xclip_says_so(self):
        with self.mock.patch.object(self.m, "IS_WINDOWS", False), self.mock.patch("subprocess.run", side_effect=FileNotFoundError):
            with self.assertRaises(self.m.ApiError) as cm:
                self.m.clipboard_get()
        self.assertEqual(cm.exception.status, 501)
        self.assertIn("xclip", str(cm.exception))

    def test_set_rejects_bad_or_huge_text(self):
        with self.assertRaises(self.m.ApiError):
            self.m.clipboard_set({"text": 5})
        with self.assertRaises(self.m.ApiError) as cm:
            self.m.clipboard_set({"text": "x" * (self.m.CLIP_MAX + 1)})
        self.assertEqual(cm.exception.status, 413)

    def test_windows_branch_uses_the_win32_clipboard(self):
        with self.mock.patch.object(self.m, "IS_WINDOWS", True), \
                self.mock.patch.object(self.m, "_win_clip_get", return_value="MT5 metni") as g, \
                self.mock.patch.object(self.m, "_win_clip_set") as st:
            self.assertEqual(self.m.clipboard_get(), {"text": "MT5 metni"})
            self.m.clipboard_set({"text": "Mac"})
        g.assert_called_once()
        st.assert_called_once_with("Mac")

    def test_empty_clipboard_is_none(self):
        with self.mock.patch.object(self.m, "IS_WINDOWS", True), self.mock.patch.object(self.m, "_win_clip_get", return_value=""):
            self.assertEqual(self.m.clipboard_get(), {"text": None})

if __name__ == "__main__":
    unittest.main()
