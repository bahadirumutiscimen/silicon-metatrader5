"""Tests for runtime/windows/supervisor.py (FakePlatform + fake clock; runs on Linux)."""
import io
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "windows"))
import supervisor as sv  # noqa: E402


class FakePlatform(sv.Platform):
    def __init__(self):
        self.next_pid = 100
        self.spawned = []   # (argv, cwd, log, pid)
        self.running = set()
        self.killed = []
        self.maximized = 0
        self.maximized_pids = []
        self.resolution = []
        self.resolution_ok = True

    def spawn(self, argv, cwd, log, env=None):
        self.next_pid += 1
        self.spawned.append((list(argv), cwd, log, self.next_pid, env))
        self.running.add(self.next_pid)
        return self.next_pid

    def alive(self, pid):
        return pid in self.running

    def kill_tree(self, pid):
        self.killed.append(pid)
        self.running.discard(pid)

    def maximize(self, pid, title_substr):
        self.maximized += 1
        self.maximized_pids.append(pid)
        return True

    def set_resolution(self, w, h):
        self.resolution.append((w, h))
        return self.resolution_ok

    def pid_of(self, idx):
        return self.spawned[idx][3]

    def name_pid(self, token):
        pids = [s[3] for s in self.spawned if any(token in a for a in s[0])]
        return pids[-1]


class Clock:
    def __init__(self):
        self.t = 1000.0

    def __call__(self):
        return self.t

    def sleep(self, s):
        self.t += s


class Base(unittest.TestCase):
    def make(self, root=None, extra_env=None):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name) if root is None else Path(tmp.name) / root
        self.root.mkdir(parents=True, exist_ok=True)
        self.mt5_dir = Path(tmp.name) / "MetaTrader 5"
        self.mt5_dir.mkdir()
        env = {"SMT5_BRIDGE_PORT": "8001", "SMT5_STATUS_PORT": "8002",
               "SMT5_DISPLAY_PORT": "18081", "SMT5_API_TOKEN": "tok123",
               "SMT5_MT5_DIR": str(self.mt5_dir)}
        env.update(extra_env or {})
        self.plat = FakePlatform()
        self.clock = Clock()
        self.sup = sv.Supervisor(self.root, env, self.plat, clock=self.clock, sleep=self.clock.sleep)
        self.state = self.root / "state"
        return self.sup

    def state_json(self):
        return json.loads((self.state / "state.json").read_text(encoding="utf-8"))


class SupervisorTests(Base):
    def test_start_order(self):
        sup = self.make()
        sup.step()
        argvs = [s[0] for s in self.plat.spawned]
        self.assertEqual(len(argvs), 3)
        self.assertIn("terminal64.exe", argvs[0][0])
        self.assertEqual(argvs[0][1:], ["/portable", "/config:mt5cfg.ini"])
        self.assertEqual(argvs[1][1:], ["-m", "siliconmetatrader5.server", "--host", "127.0.0.1", "-p", "8001"])
        self.assertTrue(argvs[2][1].endswith("smt5_status.py"))
        # websockify runs in its own task (SiliconMT5-Display), not here
        self.assertFalse(any("websockify" in a for argv in argvs for a in argv))
        self.assertNotEqual(self.state_json()["phase"], "running")  # not in the spawn step
        sup.step()
        self.assertEqual(self.state_json()["phase"], "running")

    def test_status_env_and_no_pid_files(self):
        sup = self.make()
        sup.step()
        env = self.plat.spawned[2][4]
        self.assertEqual(env["SMT5_STATUS_LISTEN"], "127.0.0.1")
        self.assertEqual(env["SMT5_STATUS_PORT_INTERNAL"], "8002")
        self.assertEqual(env["SMT5_BRIDGE_PORT_INTERNAL"], "8001")
        self.assertEqual(env["SMT5_API_TOKEN"], "tok123")
        self.assertEqual(env["SMT5_DISPLAY"], "novnc")
        self.assertEqual(env["SMT5_PLATFORM"], "windows")
        self.assertEqual(list(self.state.glob("*.pid")), [])
        pids = json.loads((self.state / "pids.json").read_text())
        self.assertEqual(set(pids), {"mt5", "bridge", "status", "supervisor"})

    def test_backoff_doubles_and_caps(self):
        sup = self.make()
        sup.step()
        for _ in range(7):
            self.plat.running.discard(self.plat.name_pid("smt5_status"))
            sup.step()
            self.clock.t = sup.comps["status"].next_start
            sup.step()
        self.assertEqual(sup.backoff_log["status"], [1, 2, 4, 8, 16, 32, 60])

    def test_backoff_resets_after_stable_minute(self):
        sup = self.make()
        sup.step()
        self.plat.running.discard(self.plat.name_pid("smt5_status"))
        sup.step()
        self.clock.t = sup.comps["status"].next_start
        sup.step()
        self.clock.t += 61
        sup.step()
        self.plat.running.discard(self.plat.name_pid("smt5_status"))
        sup.step()
        self.assertEqual(sup.backoff_log["status"], [1, 1])

    def test_mt5_and_bridge_restart_together(self):
        sup = self.make()
        sup.step()
        mt5 = self.plat.pid_of(0)
        bridge = self.plat.pid_of(1)
        self.plat.running.discard(bridge)
        sup.step()
        self.assertIn(mt5, self.plat.killed)
        self.clock.t = sup.comps["bridge"].next_start
        sup.step()
        self.assertEqual(len(self.plat.spawned), 5)
        self.assertIn("terminal64.exe", self.plat.spawned[3][0][0])
        self.assertIn("siliconmetatrader5.server", self.plat.spawned[4][0])

    def test_stop_request_kills_all(self):
        sup = self.make()
        sup.step()
        pids = {s[3] for s in self.plat.spawned}
        (self.state / "stop.request").write_text("1")
        sup.run()
        self.assertTrue(pids <= set(self.plat.killed))
        self.assertFalse((self.state / "stop.request").exists())

    def test_maintenance(self):
        sup = self.make()
        sup.step()
        mt5, bridge = self.plat.pid_of(0), self.plat.pid_of(1)
        (self.state / "maintenance.request").write_text("update mt5", encoding="utf-8")
        sup.step()
        self.assertIn(mt5, self.plat.killed)
        self.assertIn(bridge, self.plat.killed)
        self.assertTrue((self.state / "maintenance.ready").exists())
        self.assertEqual(self.state_json()["phase"], "maintenance")
        self.assertEqual(self.state_json()["detail"], "update mt5")
        self.clock.t += 120
        sup.step()
        self.assertEqual(len(self.plat.spawned), 3)  # stays down
        (self.state / "maintenance.request").unlink()
        sup.step()
        self.assertFalse((self.state / "maintenance.ready").exists())
        self.assertEqual(len(self.plat.spawned), 5)
        sup.step()
        self.assertEqual(self.state_json()["phase"], "running")

    def test_root_with_spaces(self):
        sup = self.make(root="C:\\Users\\Bahad\u0131r Umut\\AppData\\Local\\siliconmetatrader5")
        sup.step()
        r = str(self.root)
        self.assertIn("Bahad\u0131r Umut", r)
        for argv, cwd, log, _, _ in self.plat.spawned:
            self.assertTrue(all(isinstance(a, str) for a in argv))
        self.assertTrue(self.plat.spawned[2][0][1].startswith(r))
        self.assertTrue(self.plat.spawned[1][2].startswith(r))  # log path

    def test_state_json_schema(self):
        sup = self.make()
        sup.step()
        st = self.state_json()
        self.assertIn("phase", st)
        self.assertIsInstance(st["updated"], float)
        self.assertEqual(st["components"]["mt5"]["pid"], self.plat.pid_of(0))

    def test_state_json_not_rewritten_while_unchanged(self):
        sup = self.make()
        sup.step()
        sup.step()
        first = self.state_json()
        self.assertEqual(first["phase"], "running")
        for _ in range(5):
            self.clock.sleep(2)
            sup.step()
        self.assertEqual(self.state_json()["updated"], first["updated"])

    def test_phases_observable(self):
        sup = self.make()
        seen = []
        orig = sup.write_state

        def spy(phase=None, detail=None):
            orig(phase, detail)
            seen.append(sup.phase)
        sup.write_state = spy
        sup.step()
        sup.step()
        order = [p for i, p in enumerate(seen) if i == 0 or p != seen[i - 1]]
        self.assertEqual(order, ["starting_terminal", "starting_bridge", "running"])

    def test_restart_mt5_request_restarts_group(self):
        sup = self.make()
        sup.step()
        sup.step()
        mt5, bridge = self.plat.pid_of(0), self.plat.pid_of(1)
        (self.state / "restart-mt5.request").write_text("1")
        sup.step()
        self.assertIn(mt5, self.plat.killed)
        self.assertIn(bridge, self.plat.killed)
        self.assertFalse((self.state / "restart-mt5.request").exists())
        self.assertEqual(len(self.plat.spawned), 5)
        self.assertIn("terminal64.exe", self.plat.spawned[3][0][0])
        self.assertIn("siliconmetatrader5.server", self.plat.spawned[4][0])

    def test_restart_bridge_request_restarts_bridge_only(self):
        sup = self.make()
        sup.step()
        sup.step()
        mt5, bridge = self.plat.pid_of(0), self.plat.pid_of(1)
        (self.state / "restart-bridge.request").write_text("1")
        sup.step()
        self.assertIn(bridge, self.plat.killed)
        self.assertNotIn(mt5, self.plat.killed)
        self.assertFalse((self.state / "restart-bridge.request").exists())
        self.assertEqual(len(self.plat.spawned), 4)
        self.assertIn("siliconmetatrader5.server", self.plat.spawned[3][0])

    def test_maintenance_retries_when_kill_fails(self):
        sup = self.make()
        sup.step()
        orig = self.plat.kill_tree
        self.plat.kill_tree = lambda pid: self.plat.killed.append(pid)  # kill has no effect
        (self.state / "maintenance.request").write_text("x")
        sup.step()
        self.assertFalse((self.state / "maintenance.ready").exists())
        self.plat.kill_tree = orig
        sup.step()
        self.assertTrue((self.state / "maintenance.ready").exists())
        self.assertEqual(len(self.plat.spawned), 3)

    def test_partner_not_killed_when_already_dead(self):
        sup = self.make()
        sup.step()
        self.plat.running.discard(self.plat.pid_of(0))
        self.plat.running.discard(self.plat.pid_of(1))
        sup.step()
        self.assertEqual(self.plat.killed, [])

    def test_run_survives_step_exception(self):
        sup = self.make()
        orig = sup.step
        calls = []

        def flaky():
            calls.append(1)
            if len(calls) == 1:
                raise RuntimeError("boom")
            (self.state / "stop.request").write_text("1")
            orig()
        sup.step = flaky
        sup.run()
        self.assertEqual(len(calls), 2)
        self.assertIn("boom", (self.root / "logs" / "supervisor.log").read_text())

    def test_resolution_once_and_failure_logged(self):
        sup = self.make(extra_env={"SMT5_RESOLUTION": "1600x900"})
        self.plat.resolution_ok = False
        sup.step()
        sup.step()
        self.assertEqual(self.plat.resolution, [(1600, 900)])
        self.assertIn("resolution", (self.root / "logs" / "supervisor.log").read_text())

    def test_maximize_window_60s(self):
        sup = self.make()
        sup.step()
        for _ in range(40):
            self.clock.sleep(2)
            sup.step()
        self.assertTrue(10 < self.plat.maximized <= 31)
        n = self.plat.maximized
        sup.step()
        self.assertEqual(self.plat.maximized, n)

    def test_maximize_only_the_mt5_process(self):
        sup = self.make()
        sup.step()
        self.clock.sleep(2)
        sup.step()
        self.assertTrue(self.plat.maximized_pids)
        self.assertEqual(set(self.plat.maximized_pids), {self.plat.pid_of(0)})

    def test_stale_stop_request_removed_at_start(self):
        sup = self.make()
        old = self.state / "stop.request"
        old.write_text("1")
        past = time.time() - 3600
        os.utime(str(old), (past, past))
        sleeps = []

        def sleep(s):
            sleeps.append(s)
            if len(sleeps) == 2:  # a stop written while running is obeyed
                (self.state / "stop.request").write_text("1")
        sup.sleep = sleep
        sup.run()
        self.assertEqual(len(self.plat.spawned), 3, "the stale request did not stop the supervisor")
        self.assertFalse((self.state / "stop.request").exists())
        self.assertIn("stale stop.request", (self.root / "logs" / "supervisor.log").read_text())

    def test_fresh_stop_request_still_stops(self):
        sup = self.make()
        (self.state / "stop.request").write_text("1")
        os.utime(str(self.state / "stop.request"), (time.time() + 5, time.time() + 5))
        sup.run()
        self.assertEqual(self.plat.spawned, [])


TEMPLATE = """; comment
[Charts]
MaxBars=100000
PrintColor=0

[Experts]
; Enabled = the toolbar button
Enabled=1
AllowLiveTrading=1
AllowDllImport=0
Account=0
"""


class Mt5CfgTests(Base):
    """supervisor.py writes <MT5>/mt5cfg.ini like native.sh sync_mt5cfg."""

    def setup_template(self, **env):
        sup = self.make(extra_env=env)
        cfg = self.root / "runtime" / "config"
        cfg.mkdir(parents=True)
        (cfg / "mt5cfg.ini").write_text(TEMPLATE, encoding="utf-8")
        return sup

    def ini(self):
        return (self.mt5_dir / "mt5cfg.ini").read_text(encoding="utf-8")

    def test_render_mirrors_native_sh(self):
        out = sv.render_mt5cfg(TEMPLATE, {"SMT5_MAXBARS": "50000", "SMT5_ALGO_TRADING": "off", "SMT5_DLL_IMPORT": "yes"})
        self.assertIn("MaxBars=50000\n", out)
        self.assertIn("Enabled=0\nAllowLiveTrading=0\nAllowDllImport=1\n", out)
        self.assertIn("; Enabled = the toolbar button\n", out)  # comments untouched
        self.assertTrue(out.endswith("Account=0\n"))
        # invalid / missing values leave the template as it is
        same = sv.render_mt5cfg(TEMPLATE, {"SMT5_MAXBARS": "lots", "SMT5_ALGO_TRADING": "maybe"})
        self.assertEqual(same, TEMPLATE)
        for v, want in (("1", "1"), ("true", "1"), ("YES", "1"), ("on", "1"), ("0", "0"), ("false", "0"), ("no", "0")):
            self.assertIn("AllowDllImport=%s\n" % want, sv.render_mt5cfg(TEMPLATE, {"SMT5_DLL_IMPORT": v}), v)

    def test_written_before_mt5_starts(self):
        sup = self.setup_template(SMT5_MAXBARS="25000", SMT5_ALGO_TRADING="0")
        sup.step()
        ini = self.ini()
        self.assertIn("MaxBars=25000", ini)
        self.assertIn("Enabled=0", ini)
        self.assertIn("synchronized", (self.root / "logs" / "supervisor.log").read_text())

    def test_rewritten_only_when_the_result_changes(self):
        sup = self.setup_template(SMT5_MAXBARS="25000")
        sup.step()
        (self.mt5_dir / "mt5cfg.ini").write_text(self.ini() + "Mine=1\n", encoding="utf-8")
        self.plat.running.discard(self.plat.pid_of(0))  # MT5 exits, is started again
        sup.step()
        self.clock.t = sup.comps["mt5"].next_start
        sup.step()
        self.assertIn("Mine=1", self.ini(), "unchanged .env: the file in the MT5 folder stays")
        sup.env["SMT5_MAXBARS"] = "30000"
        self.plat.running.discard(self.plat.name_pid("terminal64"))
        sup.step()
        self.clock.t = sup.comps["mt5"].next_start
        sup.step()
        self.assertIn("MaxBars=30000", self.ini())
        self.assertNotIn("Mine=1", self.ini())

    def test_missing_template_does_not_block_mt5(self):
        sup = self.make()
        sup.step()
        self.assertIn("terminal64.exe", self.plat.spawned[0][0][0])


class LogRotationTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)

    def test_max_bytes_like_linux(self):
        self.assertEqual(sv.log_max_bytes({}), 10 * 1024 * 1024)
        self.assertEqual(sv.log_max_bytes({"SMT5_LOG_MAX_MB": "3"}), 3 * 1024 * 1024)
        self.assertEqual(sv.log_max_bytes({"SMT5_LOG_MAX_MB": "x"}), 10 * 1024 * 1024)
        self.assertEqual(sv.log_max_bytes({"SMT5_LOG_MAX_MB": "0"}), 1 * 1024 * 1024)

    def test_pump_rotates_keeping_the_newest_part(self):
        log = self.dir / "bridge.log"
        log.write_bytes(b"old\n")
        data = b"".join(b"line %04d\n" % i for i in range(100))  # 1000 bytes
        pump = sv.LogPump(io.BytesIO(data), str(log), max_bytes=300, chunk=64)
        pump.run()
        cur, prev = log.read_bytes(), (self.dir / "bridge.log.1").read_bytes()
        self.assertLessEqual(len(cur), 300 + 64)
        self.assertLessEqual(len(prev), 300)
        self.assertTrue(data.endswith(cur), "current file holds the newest output")
        self.assertTrue((prev + cur).endswith(b"line 0099\n"))
        self.assertFalse((self.dir / "bridge.log.1.tmp").exists())

    def test_pump_keeps_draining_when_the_log_cannot_be_written(self):
        src = io.BytesIO(b"x" * 5000)
        pump = sv.LogPump(src, str(self.dir / "missing-dir" / "a.log"), max_bytes=300, chunk=64)
        pump.run()
        self.assertEqual(src.tell(), 5000, "the child's pipe is read to the end")

    def test_rotate_file(self):
        f = self.dir / "supervisor.log"
        f.write_bytes(b"a" * 50 + b"b" * 50)
        self.assertFalse(sv.rotate_file(str(f), 200))
        self.assertTrue(sv.rotate_file(str(f), 60))
        self.assertEqual(f.read_bytes(), b"")
        self.assertEqual((self.dir / "supervisor.log.1").read_bytes(), b"a" * 10 + b"b" * 50)

    def test_supervisor_log_rotates(self):
        root = self.dir / "r"
        sup = sv.Supervisor(root, {"SMT5_LOG_MAX_MB": "1", "SMT5_MT5_DIR": str(self.dir)}, FakePlatform(), clock=Clock(), sleep=lambda s: None)
        (root / "logs" / "supervisor.log").write_bytes(b"x" * (1024 * 1024 + 10))
        sup.log("after")
        self.assertTrue((root / "logs" / "supervisor.log.1").exists())
        self.assertIn(b"after", (root / "logs" / "supervisor.log").read_bytes())


class DisplayLauncherTests(unittest.TestCase):
    def setUp(self):
        import display
        self.d = display

    def test_websockify_argv(self):
        root = Path("C:/Users/Bahad\u0131r Umut/AppData/Local/siliconmetatrader5")
        argv = self.d.websockify_argv(root, ["--web", str(root / "novnc"), "127.0.0.1:6081", "127.0.0.1:7081"])
        self.assertEqual(argv[0], str(root / "python" / "python.exe"))
        self.assertEqual(argv[1:3], ["-m", "websockify"])
        self.assertEqual(argv[3:], ["--web", str(root / "novnc"), "127.0.0.1:6081", "127.0.0.1:7081"])

    def test_loopback_only(self):
        root = Path("/r")
        for bad in (["0.0.0.0:6081", "127.0.0.1:7081"], ["6081", "127.0.0.1:7081"], ["127.0.0.1:6081", "203.0.113.5:7081"], ["127.0.0.1:6081"]):
            with self.assertRaises(ValueError, msg=bad):
                self.d.websockify_argv(root, ["--web", "/r/novnc"] + bad)

    def test_log_limit_from_the_task_arguments(self):
        # SYSTEM cannot read the user-only .env: the limit comes with the task's arguments
        web = ["--web", "/r/novnc", "127.0.0.1:6081", "127.0.0.1:7081"]
        self.assertEqual(self.d.split_args(["--log-max-mb", "3"] + web), (3 * 1024 * 1024, web))
        self.assertEqual(self.d.split_args(web), (10 * 1024 * 1024, web))
        self.assertEqual(self.d.split_args(["--log-max-mb", "x"] + web), (10 * 1024 * 1024, web))
        self.assertEqual(self.d.split_args(["--log-max-mb", "0"] + web), (1024 * 1024, web))
        src = Path(sv.__file__).with_name("display.py").read_text(encoding="utf-8")
        self.assertNotIn("load_env", src, "display.py must not read .env")

    def test_restart_backoff(self):
        self.assertEqual([self.d.backoff(n) for n in range(8)], [2, 4, 8, 16, 32, 60, 60, 60])



if __name__ == "__main__":
    unittest.main()
