import time
import unittest
from datetime import timezone
from types import SimpleNamespace

import siliconmetatrader5
from siliconmetatrader5 import bar_time, format_offset, server_now, server_time_offset


class _FakeMT5:
    """symbol -> tick epoch on the broker clock (None = no tick, KeyError = unknown symbol)."""

    def __init__(self, ticks):
        self.ticks = ticks

    def symbol_select(self, symbol, enable):
        if symbol not in self.ticks:
            raise KeyError(symbol)
        return True

    def symbol_info_tick(self, symbol):
        t = self.ticks.get(symbol)
        return None if t is None else SimpleNamespace(time=t)


class ServerTimeTest(unittest.TestCase):
    def test_version_is_single_sourced(self):
        self.assertEqual(siliconmetatrader5.__version__, "1.3.0")

    def test_offset_from_live_tick(self):
        now = time.time()
        mt5 = _FakeMT5({"EURUSD": int(now) + 2 * 3600 - 5})
        self.assertEqual(server_time_offset(mt5), 7200)
        self.assertEqual(format_offset(7200), "UTC+2")

    def test_fallback_symbol_when_market_closed(self):
        now = time.time()
        mt5 = _FakeMT5({"EURUSD": int(now) - 3 * 86400, "BTCUSD": int(now) + 3 * 3600})
        self.assertEqual(server_time_offset(mt5), 3 * 3600)

    def test_unknown_offset_and_server_now_fallback(self):
        last = int(time.time()) - 86400
        mt5 = _FakeMT5({"EURUSD": last})
        self.assertIsNone(server_time_offset(mt5, fallbacks=()))
        self.assertEqual(int(server_now(mt5).timestamp()), last)  # newest tick time
        with self.assertRaises(RuntimeError):
            server_now(_FakeMT5({"EURUSD": None}), offset=None)

    def test_server_now_is_on_server_clock(self):
        dt = server_now(_FakeMT5({}), offset=-16200)
        self.assertEqual(dt.tzinfo, timezone.utc)
        self.assertAlmostEqual(dt.timestamp(), time.time() - 16200, delta=2)
        self.assertEqual(format_offset(-16200), "UTC-4:30")
        self.assertEqual(format_offset(None), "unknown")
        self.assertEqual(bar_time(0).year, 1970)


if __name__ == "__main__":
    unittest.main()
