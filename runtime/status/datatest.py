"""Data test for the setup wizard: does real market data flow through the bridge?

Runs inside the container (the user needs no local Python) and mirrors
examples/fetch_data.py: initialize → account/terminal → symbol → the three
copy_rates_* methods → a few seconds of live ticks, all measured on the
*broker's* clock (MT5 bar times are server wall-clock times).

Bars and ticks are converted to plain tuples on the Windows side via the
bridge's eval(), so the Linux side needs no numpy.
"""
from __future__ import annotations

import re
import time
from datetime import datetime, timezone
from typing import Any, Callable, Dict, List, Optional, Tuple

TIMEFRAMES = {"M1": 1, "M5": 5, "M15": 15, "M30": 30, "H1": 60, "H4": 240, "D1": 1440}
SYMBOL_RE = re.compile(r"^[A-Za-z0-9#][A-Za-z0-9._#+&!-]{0,31}$")
FALLBACK_SYMBOLS = ("BTCUSD", "ETHUSD", "XAUUSD")
OFFSET_STEP = 30 * 60
MAX_TICK_AGE = 120
RANGE_HOURS = 6        # minimum copy_rates_range window
RANGE_BARS = 5         # …and at least this many bars of the timeframe (D1: 5 days, covers a weekend)

Bar = Tuple[int, float, float, float, float, int]      # time, open, high, low, close, tick_volume
Tick = Tuple[int, int, float, float]                   # time, time_msc, bid, ask


class Ops:
    """What the data test needs from MT5. BridgeOps talks to the real bridge."""

    def initialize(self) -> Tuple[bool, str]: ...
    def terminal(self) -> Optional[Dict[str, Any]]: ...
    def account(self) -> Optional[Dict[str, Any]]: ...
    def select(self, symbol: str) -> bool: ...
    def symbol_info(self, symbol: str) -> Optional[Tuple[int, float]]: ...   # digits, point
    def tick(self, symbol: str) -> Optional[Tick]: ...
    def rates_from_pos(self, symbol: str, tf: str, count: int) -> List[Bar]: ...
    def rates_from(self, symbol: str, tf: str, ts: int, count: int) -> List[Bar]: ...
    def rates_range(self, symbol: str, tf: str, start: int, end: int) -> List[Bar]: ...
    def market_watch(self) -> Tuple[List[str], int]: ...   # visible symbol names, total symbols
    # speed test (bench): seconds measured inside Windows Python unless noted
    def cpu_seconds(self, loops: int) -> float: ...
    def rates_timed(self, symbol: str, bars: int) -> Tuple[int, float]: ...
    def tick_roundtrip(self, symbol: str) -> None: ...      # one bridge round trip, timed by the caller
    def close(self) -> None: ...


_REMOTE_HELPERS = '''
def _smt5_rows(a):
    if a is None:
        return []
    return [(int(r["time"]), float(r["open"]), float(r["high"]), float(r["low"]), float(r["close"]), int(r["tick_volume"])) for r in a]
def _smt5_tick(t):
    return None if t is None else (int(t.time), int(t.time_msc), float(t.bid), float(t.ask))
def _smt5_sym(s):
    return None if s is None else (int(s.digits), float(s.point))
def _smt5_cpu(n):
    import time
    t = time.perf_counter()
    s = 0
    for i in range(n):
        s += i * i % 7
    return time.perf_counter() - t
def _smt5_rates_timed(sym, n):
    import time
    t = time.perf_counter()
    a = mt5.copy_rates_from_pos(sym, mt5.TIMEFRAME_M1, 0, n)
    return (0 if a is None else len(a)), time.perf_counter() - t
def _smt5_watch():
    allsyms = mt5.symbols_get() or ()
    return [str(s.name) for s in allsyms if s.visible], len(allsyms)
def _smt5_server_now():
    import time
    best = 0
    for s in (mt5.symbols_get() or ())[:400]:
        if s.visible:
            t = mt5.symbol_info_tick(s.name)
            if t is not None and t.time > best:
                best = int(t.time)
    return best or int(time.time())
def _smt5_deals(a, b):
    d = mt5.history_deals_get(a, b)
    if d is None:
        return []
    return [(int(x.time), int(x.type), int(x.entry), float(x.profit), float(x.commission), float(x.swap),
             float(getattr(x, "fee", 0.0)), str(x.symbol), float(x.volume), int(x.position_id)) for x in d]
'''


class BridgeOps(Ops):
    def __init__(self, host: str, port: int):
        import rpyc
        from siliconmetatrader5 import MetaTrader5

        self._obtain = rpyc.utils.classic.obtain
        self.c = MetaTrader5(host=host, port=port)
        self.c.execute(_REMOTE_HELPERS)

    def _eval(self, expr: str):
        return self._obtain(self.c.eval(expr))

    def initialize(self):
        ok = bool(self.c.initialize())
        return ok, "" if ok else str(self.c.last_error())

    def terminal(self):
        ti = self.c.terminal_info()
        return None if ti is None else {"connected": bool(ti.connected), "build": int(ti.build)}

    def account(self):
        ai = self.c.account_info()
        if ai is None:
            return None
        login = str(ai.login or "")
        return {"server": ai.server, "login_masked": ("•••" + login[-3:]) if login else None,
                "trade_mode": {0: "demo", 1: "contest", 2: "real"}.get(ai.trade_mode, "unknown")}

    def select(self, symbol):
        return bool(self.c.symbol_select(symbol, True))

    def symbol_info(self, symbol):
        return self._eval(f"_smt5_sym(mt5.symbol_info({symbol!r}))")

    def tick(self, symbol):
        return self._eval(f"_smt5_tick(mt5.symbol_info_tick({symbol!r}))")

    def rates_from_pos(self, symbol, tf, count):
        return self._eval(f"_smt5_rows(mt5.copy_rates_from_pos({symbol!r}, mt5.TIMEFRAME_{tf}, 0, {int(count)}))")

    def rates_from(self, symbol, tf, ts, count):
        return self._eval(f"_smt5_rows(mt5.copy_rates_from({symbol!r}, mt5.TIMEFRAME_{tf}, {int(ts)}, {int(count)}))")

    def rates_range(self, symbol, tf, start, end):
        return self._eval(f"_smt5_rows(mt5.copy_rates_range({symbol!r}, mt5.TIMEFRAME_{tf}, {int(start)}, {int(end)}))")

    def server_now(self):
        return int(self._eval("_smt5_server_now()"))

    def deals(self, start, end):
        return self._eval(f"_smt5_deals({int(start)}, {int(end)})")

    def market_watch(self):
        if not self.c.initialize():
            return [], 0
        names, total = self._eval("_smt5_watch()")
        return list(names), int(total)

    def cpu_seconds(self, loops):
        return float(self._eval(f"_smt5_cpu({int(loops)})"))

    def rates_timed(self, symbol, bars):
        rows, secs = self._eval(f"_smt5_rates_timed({symbol!r}, {int(bars)})")
        return int(rows), float(secs)

    def tick_roundtrip(self, symbol):
        self.c.symbol_info_tick(symbol)

    def close(self):
        self.c.close()  # never remote_shutdown: the terminal is shared


def market_watch(ops: Ops) -> Dict[str, Any]:
    """Symbols shown in MT5's Market Watch (what the user trades), in its order."""
    names, total = ops.market_watch()
    return {"symbols": names, "total": total}


BENCH_CPU_LOOPS = 2_000_000
BENCH_TICKS = 50


def bench(ops: Ops, symbol: str = "EURUSD", bars: int = 50_000, clock: Callable[[], float] = time.perf_counter) -> Dict[str, Any]:
    """Speed test for comparing runtime modes (QEMU vs Rosetta): Windows-side
    CPU, a large copy_rates_from_pos, bar transfer through the bridge and the
    bridge round-trip latency. Every number is "lower is better"."""
    out: Dict[str, Any] = {"symbol": symbol, "bars_requested": bars, "ok": False, "error": None}
    try:
        ok, err = ops.initialize()
        if not ok:
            raise RuntimeError(f"initialize() failed: {err}")
        ops.select(symbol)
        out["cpu_s"] = round(ops.cpu_seconds(BENCH_CPU_LOOPS), 3)
        # copy_rates_from_pos returns None (0 rows) when the request reaches
        # MaxBars or the downloaded history (100 000 with MaxBars=100000 fails,
        # 25 000 works): halve until bars come back.
        rows, secs, n = 0, 0.0, bars
        for n in (bars, bars // 2, bars // 4):
            rows, secs = ops.rates_timed(symbol, max(1_000, n))
            if rows:
                break
        out["rates_rows"], out["rates_s"] = rows, round(secs, 3)
        t = clock()
        moved = ops.rates_from_pos(symbol, "M1", 10_000)
        out["transfer_rows"], out["transfer_s"] = len(moved), round(clock() - t, 3)
        t = clock()
        for _ in range(BENCH_TICKS):
            ops.tick_roundtrip(symbol)
        out["tick_ms"] = round((clock() - t) * 1000.0 / BENCH_TICKS, 2)
        out["ok"] = True
    except Exception as exc:  # noqa: BLE001 — report what the bridge throws
        out["error"] = f"{type(exc).__name__}: {exc}"
    return out


def range_seconds(tf: str) -> int:
    return max(RANGE_HOURS * 3600, RANGE_BARS * TIMEFRAMES[tf] * 60)


def validate(symbol: str, tf: str, count: int, tick_seconds: float) -> None:
    if not SYMBOL_RE.match(symbol or ""):
        raise ValueError(f"invalid symbol: {symbol!r}")
    if tf not in TIMEFRAMES:
        raise ValueError(f"tf must be one of {sorted(TIMEFRAMES)}")
    if not 10 <= count <= 1000:
        raise ValueError("count must be between 10 and 1000")
    if not 0 <= tick_seconds <= 15:
        raise ValueError("ticks must be between 0 and 15 seconds")


def _offset(tick_time: int, now_utc: float) -> Optional[int]:
    raw = tick_time - now_utc
    rounded = int(round(raw / OFFSET_STEP)) * OFFSET_STEP
    if abs(raw - rounded) > MAX_TICK_AGE or abs(rounded) > 14 * 3600:
        return None
    return rounded


def _fmt_offset(offset: Optional[int]) -> str:
    if offset is None:
        return "unknown"
    h, m = divmod(abs(offset) // 60, 60)
    return f"UTC{'+' if offset >= 0 else '-'}{h}" + (f":{m:02d}" if m else "")


def _iso(epoch: int) -> str:
    return datetime.fromtimestamp(int(epoch), tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%S")


class _Stop(Exception):
    pass


def run(ops: Ops, symbol: str = "EURUSD", tf: str = "M5", count: int = 100, tick_seconds: float = 5.0,
        clock: Callable[[], float] = time.time, sleep: Callable[[float], None] = time.sleep) -> Dict[str, Any]:
    """Run the data test. Never raises for MT5 problems: they end up in the result."""
    validate(symbol, tf, count, tick_seconds)
    checks: List[Dict[str, Any]] = []
    result: Dict[str, Any] = {
        "symbol": symbol, "timeframe": tf, "checks": checks, "methods": {}, "bars": [],
        "ticks": None, "server_offset": None, "server_offset_s": None, "server_now": None,
        "verdict": "error", "message": "",
    }

    def step(cid: str, title: str, fn):
        """fn() -> (ok, detail, value)"""
        t0 = time.monotonic()
        try:
            ok, detail, value = fn()
        except Exception as exc:  # noqa: BLE001 — the bridge can raise anything
            ok, detail, value = False, f"{type(exc).__name__}: {exc}", None
        checks.append({"id": cid, "title": title, "ok": ok, "detail": detail,
                       "ms": round((time.monotonic() - t0) * 1000)})
        if not ok:
            raise _Stop(f"{title}: {detail}")
        return value

    def do_init():
        ok, err = ops.initialize()
        return ok, "connected" if ok else f"initialize() failed: {err}", None

    def do_account():
        term, acct = ops.terminal(), ops.account()
        if not term or not term.get("connected"):
            return False, "terminal is not connected to the broker (log in via the MT5 desktop)", None
        if not acct:
            return False, "no account logged in", None
        return True, f"{acct['server']} {acct['login_masked']} ({acct['trade_mode']}), build {term.get('build')}", acct

    def do_select():
        if not ops.select(symbol):
            return False, f"{symbol} not available (check the exact name in Market Watch, e.g. {symbol}.m)", None
        return True, "in Market Watch", None

    def do_clock():
        candidates = [symbol] + [s for s in FALLBACK_SYMBOLS if s != symbol]
        last_tick = None
        for sym in candidates:
            if sym != symbol and not ops.select(sym):
                continue
            tk = ops.tick(sym)
            if not tk:
                continue
            last_tick = last_tick or tk
            off = _offset(tk[0], clock())
            if off is not None:
                return True, f"broker {_fmt_offset(off)} (from {sym})", (off, None)
        if last_tick:
            return True, "no live tick in the last 2 min (market closed?) — using last tick time", (None, last_tick[0])
        return False, "no ticks at all for the symbol", None

    def method(name: str, rows: List[Bar], now_server: int) -> Tuple[bool, str, Dict[str, Any]]:
        info: Dict[str, Any] = {"rows": len(rows), "first": None, "last": None, "fresh_min": None}
        if rows:
            info["first"], info["last"] = _iso(rows[0][0]), _iso(rows[-1][0])
            info["last_epoch"] = int(rows[-1][0])
            info["fresh_min"] = round((now_server - rows[-1][0]) / 60.0, 2)
        result["methods"][name] = info
        if not rows:
            return False, "no data (history may still be syncing after login)", info
        return True, f"{len(rows)} bars, last {info['last']} ({info['fresh_min']} min ago)", info

    try:
        step("initialize", "Bridge connection", do_init)
        acct = step("account", "Account & terminal", do_account)
        result["account"] = acct
        step("symbol", f"Symbol {symbol}", do_select)
        offset, tick_time = step("clock", "Broker clock", do_clock)
        result["server_offset_s"] = offset
        result["server_offset"] = _fmt_offset(offset)
        now_server = int(clock() + offset) if offset is not None else int(tick_time)
        result["server_now"] = _iso(now_server)

        pos = step("from_pos", "copy_rates_from_pos",
                   lambda: method("from_pos", ops.rates_from_pos(symbol, tf, count), now_server))
        step("from", "copy_rates_from (broker now)",
             lambda: method("from", ops.rates_from(symbol, tf, now_server, count), now_server))
        # Window scales with the timeframe and ends at the newest bar, so D1/H4
        # and a closed market (weekend) still have bars in it.
        window = range_seconds(tf)
        last_bar = int(pos["last_epoch"]) if pos.get("last_epoch") else now_server
        range_end = max(now_server, last_bar)
        hours = window // 3600
        label = f"last {hours // 24} d" if hours >= 48 else f"last {hours} h"
        step("range", f"copy_rates_range ({label})",
             lambda: method("range", ops.rates_range(symbol, tf, min(now_server, last_bar) - window, range_end), now_server))
        result["bars"] = [{"t": _iso(b[0]), "o": b[1], "h": b[2], "l": b[3], "c": b[4], "v": b[5]}
                          for b in ops.rates_from_pos(symbol, tf, min(count, 60))]

        if tick_seconds > 0:
            def do_ticks():
                info = ops.symbol_info(symbol) or (5, 0.00001)
                digits, point = info
                seen: Dict[int, Tick] = {}
                end = time.monotonic() + tick_seconds
                while True:
                    tk = ops.tick(symbol)
                    if tk:
                        seen[tk[1]] = tk
                    if time.monotonic() >= end:
                        break
                    sleep(0.5)
                samples = sorted(seen.values(), key=lambda x: x[1])
                last = samples[-1] if samples else None
                ticks = {
                    "count": len(samples), "seconds": tick_seconds, "digits": digits,
                    "bid": last[2] if last else None, "ask": last[3] if last else None,
                    "spread_points": round((last[3] - last[2]) / point, 1) if last and point else None,
                    "samples": [{"t": _iso(s[0]), "bid": s[2], "ask": s[3]} for s in samples[-10:]],
                }
                result["ticks"] = ticks
                if offset is not None and not samples:
                    return False, "no ticks received", ticks
                return True, f"{len(samples)} tick(s) in {tick_seconds:g} s", ticks
            step("ticks", "Live ticks", do_ticks)

        limit = 2 * TIMEFRAMES[tf]
        stale = [n for n, m in result["methods"].items() if m["fresh_min"] is not None and m["fresh_min"] > limit]
        if offset is None:
            result["verdict"], result["message"] = "market_closed", "Data arrives, but the market looks closed: freshness cannot be judged."
        elif stale:
            result["verdict"] = "stale"
            result["message"] = f"Data arrives but {', '.join(stale)} is older than {limit} min; history may still be syncing."
        else:
            result["verdict"], result["message"] = "ok", f"Data flow verified: all methods fresh within {limit} min."
    except _Stop as stop:
        result["verdict"] = "no_data" if checks and checks[-1]["id"] in ("from_pos", "from", "range") else "error"
        result["message"] = str(stop)
    return result
