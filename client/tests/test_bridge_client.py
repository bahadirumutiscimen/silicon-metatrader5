import datetime as dt
import threading
import time
import unittest
from unittest.mock import patch

from siliconmetatrader5 import MT5BridgeError, MetaTrader5


class _FakeMT5Module:
    def __init__(self):
        self.calls = []
        self.results = {}
        self.errors = {}
        self.delays = {}

    def __getattr__(self, name):
        def _fn(*args, **kwargs):
            self.calls.append((name, args, kwargs))
            delay = float(self.delays.get(name, 0.0))
            if delay > 0:
                time.sleep(delay)
            if name in self.errors:
                raise self.errors[name]
            return self.results.get(name, {"method": name, "args": args, "kwargs": kwargs})

        return _fn


class _FakeModules:
    def __init__(self, mt5_module):
        self.MetaTrader5 = mt5_module


class _FakeConn:
    def __init__(self, mt5_module):
        self._config = {}
        self.modules = _FakeModules(mt5_module)
        self.closed = False
        self.eval_calls = []
        self.execute_calls = []
        self.ping_ok = True
        self.ping_delay = 0.0

    def ping(self):
        if self.ping_delay > 0:
            time.sleep(self.ping_delay)
        if not self.ping_ok:
            raise RuntimeError("ping failed")
        return True

    def eval(self, code):
        self.eval_calls.append(code)
        if code == "mt5":
            return self.modules.MetaTrader5
        # eval-based calls ("mt5.positions_get(...)") honour the module's scripted errors
        name = code[4:].split("(", 1)[0] if code.startswith("mt5.") else None
        if name in self.modules.MetaTrader5.errors:
            raise self.modules.MetaTrader5.errors[name]
        return {"eval": code}

    def execute(self, code):
        self.execute_calls.append(code)

    def close(self):
        self.closed = True


class _FakeBgThread:
    instances = []

    def __init__(self, _conn):
        self.stopped = False
        _FakeBgThread.instances.append(self)

    def stop(self):
        self.stopped = True


class TestBridgeClient(unittest.TestCase):
    def setUp(self):
        _FakeBgThread.instances = []
        self.remote = _FakeMT5Module()
        self.conn = _FakeConn(self.remote)

        self.p_connect = patch("siliconmetatrader5.rpyc.classic.connect", return_value=self.conn)
        self.p_obtain = patch("siliconmetatrader5.rpyc.utils.classic.obtain", side_effect=lambda x: x)
        self.p_bg = patch("siliconmetatrader5.BgServingThread", side_effect=lambda conn: _FakeBgThread(conn))

        self.p_connect.start()
        self.p_obtain.start()
        self.p_bg.start()

    def tearDown(self):
        self.p_bg.stop()
        self.p_obtain.stop()
        self.p_connect.stop()

    def test_close_default_does_not_shutdown_remote(self):
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        mt5.close()
        called = [c[0] for c in self.remote.calls]
        self.assertNotIn("shutdown", called)
        self.assertTrue(self.conn.closed)

    def test_close_with_remote_shutdown_calls_shutdown(self):
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        mt5.close(remote_shutdown=True)
        called = [c[0] for c in self.remote.calls]
        self.assertIn("shutdown", called)
        self.assertTrue(self.conn.closed)

    def test_keepalive_thread_stops_on_close(self):
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=True, timeout=10)
        self.assertEqual(len(_FakeBgThread.instances), 1)
        bg = _FakeBgThread.instances[0]
        self.assertFalse(bg.stopped)
        mt5.close()
        self.assertTrue(bg.stopped)

    def test_copy_rates_from_pos_uses_remote_method_not_eval(self):
        self.remote.results["copy_rates_from_pos"] = [{"time": 1}]
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        result = mt5.copy_rates_from_pos("EURUSD", mt5.TIMEFRAME_M5, 0, 10)
        self.assertEqual(result, [{"time": 1}])
        self.assertEqual(self.remote.calls[0][0], "copy_rates_from_pos")
        self.assertEqual(self.conn.eval_calls, [])

    def test_datetime_normalization_for_range_calls(self):
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        dt_from = dt.datetime(2026, 3, 25, 12, 0, 0)
        dt_to = dt.datetime(2026, 3, 25, 13, 0, 0)
        mt5.copy_rates_range("EURUSD", mt5.TIMEFRAME_M5, dt_from, dt_to)
        _, args, _ = self.remote.calls[-1]
        self.assertIsInstance(args[2], int)
        self.assertIsInstance(args[3], int)
        self.assertLess(args[2], args[3])

    def test_result_expired_is_normalized(self):
        self.remote.errors["positions_get"] = RuntimeError("result expired")
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        with self.assertRaises(MT5BridgeError) as cm:
            mt5.positions_get()
        self.assertEqual(cm.exception.reason_code, "RESULT_EXPIRED")
        self.assertEqual(cm.exception.operation, "positions_get")

    def test_connection_closed_guard(self):
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        mt5.close()
        with self.assertRaises(RuntimeError):
            mt5.version()

    def test_eval_execute_still_available(self):
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=10)
        response = mt5.eval("1+1")
        mt5.execute("x=1")
        self.assertEqual(response, {"eval": "1+1"})
        # the constructor imports MetaTrader5 remotely first; the user's statement comes last
        self.assertEqual(self.conn.execute_calls[-1], "x=1")

    def test_timeout_argument_is_ignored_for_backward_compat(self):
        MetaTrader5(host="localhost", port=8001, keepalive=False, timeout=1)
        self.assertIn("sync_request_timeout", self.conn._config)
        self.assertIsNone(self.conn._config["sync_request_timeout"])

    def test_watchdog_unhealthy_on_ping_fail(self):
        self.conn.ping_ok = False
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False)
        mt5.start_watchdog(interval_sec=0.02, max_miss=2, probe_timeout_sec=0.02)
        time.sleep(0.12)
        status = mt5.health_status()
        self.assertEqual(status["status"], "unhealthy")
        self.assertGreaterEqual(status["consecutive_failures"], 2)
        self.assertEqual(status["watchdog_running"], True)
        mt5.stop_watchdog()
        mt5.close()

    def test_watchdog_detects_stuck_inflight_call(self):
        self.remote.delays["version"] = 0.35
        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False)
        mt5.start_watchdog(interval_sec=0.02, max_miss=1, stall_timeout_sec=0.05, probe_timeout_sec=0.02)

        worker = threading.Thread(target=mt5.version)
        worker.start()
        deadline = time.time() + 0.7
        status = mt5.health_status()
        while time.time() < deadline and status["status"] != "unhealthy":
            time.sleep(0.02)
            status = mt5.health_status()
        self.assertEqual(status["status"], "unhealthy")
        self.assertIn("CALL_STUCK", str(status["last_error_reason"]))

        worker.join(timeout=1.0)
        mt5.stop_watchdog()
        mt5.close()

    def test_watchdog_callback_called_on_unhealthy(self):
        self.conn.ping_ok = False
        events = []

        def _cb(snapshot):
            events.append(snapshot.get("status"))

        mt5 = MetaTrader5(host="localhost", port=8001, keepalive=False)
        mt5.start_watchdog(
            interval_sec=0.02,
            max_miss=1,
            probe_timeout_sec=0.02,
            on_unhealthy=_cb,
        )
        time.sleep(0.08)
        mt5.stop_watchdog()
        mt5.close()
        self.assertGreaterEqual(len(events), 1)


if __name__ == "__main__":
    unittest.main()
