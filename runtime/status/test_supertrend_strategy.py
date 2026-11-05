"""Tests for supertrend_strategy.py (no MT5 needed).

Run: python3 -m unittest runtime/status/test_supertrend_strategy.py -v
"""
import math
import random
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import supertrend_strategy as st  # noqa: E402


def make_bars(n=600, seed=7, start=1_700_000_000, step=3600):
    rnd = random.Random(seed)
    price, out = 1.1, []
    for i in range(n):
        drift = 0.0006 * math.sin(i / 40)                 # alternating trends
        o = price
        c = o + drift + rnd.gauss(0, 0.0012)
        h = max(o, c) + abs(rnd.gauss(0, 0.0006))
        lo = min(o, c) - abs(rnd.gauss(0, 0.0006))
        out.append(st.Bar(start + i * step, o, h, lo, c, 100))
        price = c
    return out


def reference(bars, period, mult):
    """Straight transcription of TradingView's ta.supertrend (direction: -1 = up there)."""
    atr, up_prev, lo_prev, st_prev, dirs, vals = None, None, None, None, [], []
    trs = []
    for i, b in enumerate(bars):
        tr = b.high - b.low if i == 0 else max(b.high - b.low, abs(b.high - bars[i - 1].close), abs(b.low - bars[i - 1].close))
        trs.append(tr)
        if i + 1 < period:
            dirs.append(0); vals.append(None); continue
        atr = sum(trs) / period if atr is None else (atr * (period - 1) + tr) / period
        hl2 = (b.high + b.low) / 2
        up, lo = hl2 + mult * atr, hl2 - mult * atr
        if lo_prev is not None:
            pc = bars[i - 1].close
            lo = lo if (lo > lo_prev or pc < lo_prev) else lo_prev
            up = up if (up < up_prev or pc > up_prev) else up_prev
        if st_prev is None:
            d = 1
        elif st_prev == up_prev:
            d = -1 if b.close > up else 1
        else:
            d = 1 if b.close < lo else -1
        stv = lo if d == -1 else up
        dirs.append(-d); vals.append(stv)
        up_prev, lo_prev, st_prev = up, lo, stv
    return dirs, vals


class SupertrendTest(unittest.TestCase):
    def test_matches_tradingview_reference(self):
        bars = make_bars()
        for period, mult in [(10, 3.0), (7, 2.0), (1, 1.5)]:
            ind = st.Supertrend(period, mult)
            got_d, got_v = [], []
            for b in bars:
                got_d.append(ind.update(b)); got_v.append(ind.value)
            ref_d, ref_v = reference(bars, period, mult)
            self.assertEqual(got_d, ref_d, (period, mult))
            for a, b in zip(got_v, ref_v):
                if b is None:
                    self.assertIsNone(a)
                else:
                    self.assertAlmostEqual(a, b, places=12)

    def test_signals_are_trend_changes(self):
        bars = make_bars()
        res = st.backtest(bars, st.Params(10, 3.0))
        self.assertGreater(len(res.signals), 4)
        sides = [s.side for s in res.signals]
        self.assertTrue(all(a != b for a, b in zip(sides, sides[1:])), "signals alternate")
        for s in res.signals:
            i = next(k for k, b in enumerate(bars) if b.time == s.time)
            self.assertEqual(res.trend[i], 1 if s.side == "BUY" else -1)
            self.assertEqual(res.trend[i - 1], -res.trend[i])

    def test_no_repaint_prefix_property(self):
        """Signals seen with history up to bar k never change when more bars arrive."""
        bars = make_bars()
        full = st.backtest(bars).signals
        for k in range(50, len(bars), 37):
            part = st.backtest(bars[:k]).signals
            self.assertEqual(part, [s for s in full if s.time <= bars[k - 1].time])

    def test_live_engine_equals_backtest(self):
        """Feeding bars one by one (with repeats, like polling) gives the backtest's signals."""
        bars = make_bars()
        eng, live = st.SupertrendStrategy(st.Params()), []
        for i in range(len(bars)):
            for b in bars[max(0, i - 2): i + 1]:          # each poll returns the last 3 closed bars
                s = eng.on_bar(b)
                if s:
                    live.append(s)
        self.assertEqual(live, st.backtest(bars).signals)

    def test_trades_enter_at_next_open_and_reverse(self):
        bars = make_bars()
        res = st.backtest(bars, cost=0.0001)
        by_time = {b.time: i for i, b in enumerate(bars)}
        for sig, tr in zip(res.signals, res.trades):
            i = by_time[sig.time]
            if i + 1 < len(bars):
                self.assertEqual(tr.entry_time, bars[i + 1].time)
                self.assertEqual(tr.entry_price, bars[i + 1].open)
                self.assertEqual(tr.side, sig.side)
        closed = [t for t in res.trades if not t.open]
        for a, b in zip(closed, res.trades[1:]):
            self.assertEqual(a.exit_time, b.entry_time)
            self.assertEqual(a.exit_price, b.entry_price)
            self.assertAlmostEqual(a.pnl, st._pnl(a.side, a.entry_price, a.exit_price) - 0.0001)
        s = res.stats
        self.assertEqual(s["trades"], len(closed))
        self.assertAlmostEqual(s["net"], sum(t.pnl for t in closed))
        self.assertGreaterEqual(s["max_drawdown"], 0)

    def test_rows_and_params(self):
        self.assertEqual(st.rows_to_bars([(60, 1, 2, 0.5, 1.5, 9, 0, 0)]), [st.Bar(60, 1.0, 2.0, 0.5, 1.5, 9)])
        with self.assertRaises(ValueError):
            st.SupertrendStrategy(st.Params(0, 3))
        self.assertEqual(st.backtest([]).stats["trades"], 0)


class FakeMT5:
    ACCOUNT_TRADE_MODE_DEMO, ACCOUNT_TRADE_MODE_REAL = 0, 2

    def __init__(self, mode):
        self.mode = mode

    def account_info(self):
        return type("A", (), {"trade_mode": self.mode})()


class OrderGuardTest(unittest.TestCase):
    def test_real_account_refused(self):
        with self.assertRaises(RuntimeError):
            st.send_order(FakeMT5(2), "EURUSD", "BUY")
        self.assertTrue(st.DRY_RUN)


if __name__ == "__main__":
    unittest.main()
