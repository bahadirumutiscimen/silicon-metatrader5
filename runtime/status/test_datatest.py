"""Tests for datatest.py with a fake MT5 (broker clock UTC+2).

Run: python3 -m unittest runtime/status/test_datatest.py -v
"""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import datatest  # noqa: E402

NOW = 1_793_710_000.0           # fixed "real" UTC time
OFFSET = 2 * 3600               # broker = UTC+2
SEC = {"M1": 60, "M5": 300, "M15": 900, "M30": 1800, "H1": 3600, "H4": 14400, "D1": 86400}


class FakeOps(datatest.Ops):
    def __init__(self, *, logged_in=True, symbol_ok=True, market_open=True, history=True, lag_bars=0):
        self.logged_in, self.symbol_ok, self.market_open = logged_in, symbol_ok, market_open
        self.history, self.lag_bars = history, lag_bars
        self.closed = False

    def _srv_now(self):
        return int(NOW + OFFSET - (0 if self.market_open else 3 * 86400))

    def initialize(self):
        return True, ""

    def terminal(self):
        return {"connected": self.logged_in, "build": 6295}

    def account(self):
        return {"server": "FTMO-Demo", "login_masked": "•••409", "trade_mode": "demo"} if self.logged_in else None

    def select(self, symbol):
        return self.symbol_ok and symbol == "EURUSD"

    def symbol_info(self, symbol):
        return (5, 0.00001)

    def tick(self, symbol):
        t = self._srv_now() - 2
        return (t, t * 1000, 1.11770, 1.11782)

    def _bars(self, tf, end, count):
        if not self.history:
            return []
        sec = SEC[tf]
        last = (min(end, self._srv_now()) // sec) * sec - self.lag_bars * sec
        return [(last - sec * (count - 1 - i), 1.1, 1.2, 1.0, 1.15, 10) for i in range(count)]

    def rates_from_pos(self, symbol, tf, count):
        return self._bars(tf, self._srv_now(), count)

    def rates_from(self, symbol, tf, ts, count):
        return self._bars(tf, ts, count)

    def rates_range(self, symbol, tf, start, end):
        return self._bars(tf, end, max(0, (end - start) // SEC[tf]))

    def close(self):
        self.closed = True


def run(ops, **kw):
    return datatest.run(ops, clock=lambda: NOW, sleep=lambda s: None, tick_seconds=kw.pop("ticks", 0.01), **kw)


class DataTestTest(unittest.TestCase):
    def test_ok_on_broker_clock(self):
        r = run(FakeOps())
        self.assertEqual(r["verdict"], "ok", r["message"])
        self.assertEqual(r["server_offset"], "UTC+2")
        self.assertEqual([c["id"] for c in r["checks"]],
                         ["initialize", "account", "symbol", "clock", "from_pos", "from", "range", "ticks"])
        lasts = {m["last"] for m in r["methods"].values()}
        self.assertEqual(len(lasts), 1, "all three methods must end on the same bar")
        self.assertLess(r["methods"]["from_pos"]["fresh_min"], 5)
        self.assertEqual(r["methods"]["range"]["rows"], 72)       # 6 h of M5
        self.assertEqual(r["ticks"]["spread_points"], 12.0)
        self.assertEqual(len(r["bars"]), 60)

    def test_stale_history(self):
        r = run(FakeOps(lag_bars=5))
        self.assertEqual(r["verdict"], "stale")
        self.assertIn("syncing", r["message"])

    def test_market_closed(self):
        r = run(FakeOps(market_open=False))
        self.assertEqual(r["verdict"], "market_closed")
        self.assertEqual(r["server_offset"], "unknown")

    def test_not_logged_in(self):
        r = run(FakeOps(logged_in=False))
        self.assertEqual(r["verdict"], "error")
        self.assertEqual(r["checks"][-1]["id"], "account")
        self.assertIn("log in", r["checks"][-1]["detail"])

    def test_unknown_symbol(self):
        r = run(FakeOps(symbol_ok=False))
        self.assertEqual(r["checks"][-1]["id"], "symbol")
        self.assertIn("Market Watch", r["checks"][-1]["detail"])

    def test_no_history_yet(self):
        r = run(FakeOps(history=False))
        self.assertEqual(r["verdict"], "no_data")
        self.assertEqual(r["checks"][-1]["id"], "from_pos")

    def test_validation(self):
        for kw in ({"symbol": "../x"}, {"tf": "M7"}, {"count": 5}, {"ticks": 99}):
            with self.subTest(kw=kw):
                with self.assertRaises(ValueError):
                    run(FakeOps(), **kw)

    def test_daily_and_h4_range_window(self):
        # a 6 h window holds no D1 bar: the range must scale with the timeframe
        for tf in ("D1", "H4"):
            r = run(FakeOps(), tf=tf)
            self.assertEqual(r["verdict"], "ok", (tf, r["message"]))
            self.assertGreater(r["methods"]["range"]["rows"], 0)

    def test_range_is_anchored_on_last_bar_when_market_closed(self):
        r = run(FakeOps(market_open=False))
        self.assertGreater(r["methods"]["range"]["rows"], 0)
        self.assertEqual(r["verdict"], "market_closed")

    def test_bench(self):
        class Ops(FakeOps):
            def cpu_seconds(self, loops):
                return 1.2345
            def rates_timed(self, symbol, bars):
                return bars, 0.5
            def tick_roundtrip(self, symbol):
                pass
        t = iter([0.0, 0.25, 1.0, 1.5])
        r = datatest.bench(Ops(), "EURUSD", 100_000, clock=lambda: next(t))
        self.assertTrue(r["ok"], r["error"])
        self.assertEqual(r["cpu_s"], 1.234)
        self.assertEqual(r["rates_rows"], 100_000)
        self.assertEqual(r["transfer_rows"], 10_000)
        self.assertEqual(r["transfer_s"], 0.25)
        self.assertEqual(r["tick_ms"], 10.0)  # 0.5 s / 50 round trips
        # MaxBars reached: 100k and 50k return nothing, 25k works
        class Capped(Ops):
            asked = []
            def rates_timed(self, symbol, bars):
                self.asked.append(bars)
                return (bars, 0.1) if bars <= 25_000 else (0, 0.0)
        c = Capped()
        r = datatest.bench(c, "EURUSD", 100_000, clock=lambda: 0.0)
        self.assertEqual((r["rates_rows"], c.asked), (25_000, [100_000, 50_000, 25_000]))
        bad = datatest.bench(FakeOps(logged_in=True, symbol_ok=True), "EURUSD", 1000)
        self.assertFalse(bad["ok"])  # FakeOps has no cpu_seconds → reported, not raised

    def test_market_watch(self):
        class Ops(FakeOps):
            def market_watch(self):
                return ["EURUSD", "XAUUSD"], 812
        self.assertEqual(datatest.market_watch(Ops()), {"symbols": ["EURUSD", "XAUUSD"], "total": 812})

    def test_offset_rounding(self):
        self.assertEqual(datatest._offset(int(NOW + 7200 - 3), NOW), 7200)
        self.assertEqual(datatest._offset(int(NOW + 19800 + 5), NOW), 19800)   # UTC+5:30
        self.assertIsNone(datatest._offset(int(NOW + 7200 - 600), NOW))       # 10 min old tick
        self.assertEqual(datatest._fmt_offset(19800), "UTC+5:30")


if __name__ == "__main__":
    unittest.main()
