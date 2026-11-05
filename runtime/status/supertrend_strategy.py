#!/usr/bin/env python3
"""Supertrend buy/sell strategy: an example for SiliconMetaTrader5.

Copy this file and run it next to SiliconMetaTrader5 (bridge on 127.0.0.1:8001):

    python3 -m pip install --upgrade siliconmetatrader5 numpy
    python3 supertrend_strategy.py backtest --symbol EURUSD --tf H1
    python3 supertrend_strategy.py live --symbol EURUSD --tf M5

The SiliconMetaTrader5 app runs exactly this file for its "Supertrend" page.

How it avoids repainting and stays the same in backtest and live:

* Only CLOSED bars are used. A signal is final the moment its bar closes and
  is never changed afterwards. The bar that is still forming (position 0 in
  ``copy_rates_from_pos``) is never read.
* One event-driven engine, ``SupertrendStrategy.on_bar(bar)``, produces the
  signals. The backtest replays history bar by bar through it; the live loop
  feeds it each newly closed bar. Same code, same signals.
* A signal formed on bar N is traded at the OPEN of bar N+1 (backtest) or
  right after bar N closes (live): never at a price that was not yet known.
* The engine has no side effects. Orders live in a separate block that is off
  by default (``DRY_RUN = True``) and refuses real accounts.

Supertrend = ATR bands around (high + low) / 2 with the usual "ratchet" rule,
ATR with Wilder's smoothing (the same as TradingView's ``ta.supertrend``).
Python 3.9+, standard library only (``siliconmetatrader5`` + numpy for live data).
"""

from __future__ import annotations

import argparse
import sys
import time
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence

TF_SECONDS = {"M1": 60, "M5": 300, "M15": 900, "M30": 1800, "H1": 3600, "H4": 14400, "D1": 86400}

# ----------------------------------------------------------------- data ----


@dataclass(frozen=True)
class Bar:
    """One closed bar. ``time`` is its open time in seconds (broker server clock)."""

    time: int
    open: float
    high: float
    low: float
    close: float
    volume: int = 0


@dataclass(frozen=True)
class Signal:
    """Trend change, final at the close of the bar that opened at ``time``."""

    time: int
    side: str          # "BUY" or "SELL"
    price: float       # close of the signal bar
    line: float        # Supertrend value: the natural stop level


@dataclass
class Params:
    period: int = 10
    multiplier: float = 3.0

    def check(self) -> None:
        if not 1 <= self.period <= 500:
            raise ValueError("period must be between 1 and 500")
        if not 0.1 <= self.multiplier <= 20:
            raise ValueError("multiplier must be between 0.1 and 20")


# ------------------------------------------------------------ indicator ----


class Supertrend:
    """Incremental Supertrend. Call ``update`` once per closed bar, in order."""

    def __init__(self, period: int = 10, multiplier: float = 3.0):
        self.period, self.multiplier = period, multiplier
        self.atr: Optional[float] = None
        self.upper: Optional[float] = None
        self.lower: Optional[float] = None
        self.direction = 0          # +1 up trend, -1 down trend, 0 warming up
        self.value: Optional[float] = None
        self._prev_close: Optional[float] = None
        self._tr_sum = 0.0
        self._count = 0

    def update(self, bar: Bar) -> int:
        """Feed one closed bar; returns the direction (0 while warming up)."""
        pc = self._prev_close
        tr = bar.high - bar.low if pc is None else max(bar.high - bar.low, abs(bar.high - pc), abs(bar.low - pc))
        self._count += 1
        if self.atr is None:
            self._tr_sum += tr
            if self._count < self.period:
                self._prev_close = bar.close
                return 0
            self.atr = self._tr_sum / self.period
        else:
            self.atr = (self.atr * (self.period - 1) + tr) / self.period

        mid = (bar.high + bar.low) / 2
        upper, lower = mid + self.multiplier * self.atr, mid - self.multiplier * self.atr
        if self.upper is not None and pc is not None:
            # ratchet: a band only moves towards the price unless the price broke it
            lower = lower if (lower > self.lower or pc < self.lower) else self.lower
            upper = upper if (upper < self.upper or pc > self.upper) else self.upper
        if self.direction == 0:
            direction = -1                                     # like ta.supertrend's first bar
        elif self.direction == -1:
            direction = 1 if bar.close > upper else -1
        else:
            direction = -1 if bar.close < lower else 1
        self.upper, self.lower, self.direction = upper, lower, direction
        self.value = lower if direction == 1 else upper
        self._prev_close = bar.close
        return direction


# --------------------------------------------------------------- engine ----


class SupertrendStrategy:
    """Event-driven and side-effect free: ``on_bar(closed bar) -> Signal | None``.

    The backtest and the live loop drive the same object, so they agree.
    """

    def __init__(self, params: Params = Params()):
        params.check()
        self.params = params
        self.st = Supertrend(params.period, params.multiplier)
        self.last_time: Optional[int] = None

    def on_bar(self, bar: Bar) -> Optional[Signal]:
        if self.last_time is not None and bar.time <= self.last_time:
            return None                       # seen already (live polling may repeat a bar)
        self.last_time = bar.time
        before = self.st.direction
        after = self.st.update(bar)
        if before == 0 or after == before:
            return None                       # warming up, or no trend change
        return Signal(bar.time, "BUY" if after == 1 else "SELL", bar.close, float(self.st.value))


# ------------------------------------------------------------- backtest ----


@dataclass
class Trade:
    side: str
    entry_time: int
    entry_price: float
    exit_time: Optional[int] = None
    exit_price: Optional[float] = None
    pnl: float = 0.0                  # price units, after `cost`
    open: bool = False                # still open at the last bar (marked to its close)


@dataclass
class Result:
    params: Dict[str, Any]
    bars: int
    signals: List[Signal] = field(default_factory=list)
    trades: List[Trade] = field(default_factory=list)
    line: List[Optional[float]] = field(default_factory=list)      # Supertrend value per bar
    trend: List[int] = field(default_factory=list)                 # direction per bar
    stats: Dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


def backtest(bars: Sequence[Bar], params: Params = Params(), cost: float = 0.0) -> Result:
    """Always in the market: each signal reverses the position at the next bar's open.

    ``cost`` is subtracted from every closed trade (spread + commission, price units).
    """
    strat = SupertrendStrategy(params)
    res = Result(params=asdict(params), bars=len(bars))
    pending: Optional[Signal] = None
    pos: Optional[Trade] = None
    for bar in bars:
        if pending is not None:                       # act on the previous bar's signal
            if pos is not None:
                pos.exit_time, pos.exit_price = bar.time, bar.open
                pos.pnl = _pnl(pos.side, pos.entry_price, bar.open) - cost
                res.trades.append(pos)
            pos = Trade(pending.side, bar.time, bar.open)
            pending = None
        sig = strat.on_bar(bar)
        res.line.append(strat.st.value)
        res.trend.append(strat.st.direction)
        if sig is not None:
            res.signals.append(sig)
            pending = sig
    if pos is not None and bars:
        last = bars[-1]
        pos.exit_time, pos.exit_price, pos.open = last.time, last.close, True
        pos.pnl = _pnl(pos.side, pos.entry_price, last.close)
        res.trades.append(pos)
    res.stats = stats(res.trades)
    return res


def _pnl(side: str, entry: float, exit_: float) -> float:
    return exit_ - entry if side == "BUY" else entry - exit_


def stats(trades: Iterable[Trade]) -> Dict[str, Any]:
    closed = [t for t in trades if not t.open]
    wins = [t.pnl for t in closed if t.pnl > 0]
    losses = [t.pnl for t in closed if t.pnl <= 0]
    equity, peak, max_dd, curve = 0.0, 0.0, 0.0, []
    for t in closed:
        equity += t.pnl
        peak = max(peak, equity)
        max_dd = max(max_dd, peak - equity)
        curve.append(equity)
    gross_loss = -sum(losses)
    return {
        "trades": len(closed),
        "wins": len(wins),
        "win_rate": (len(wins) / len(closed)) if closed else None,
        "net": sum(t.pnl for t in closed),
        "gross_profit": sum(wins),
        "gross_loss": gross_loss,
        "profit_factor": (sum(wins) / gross_loss) if gross_loss > 0 else None,
        "avg_trade": (sum(t.pnl for t in closed) / len(closed)) if closed else None,
        "max_drawdown": max_dd,
        "equity": curve,
    }


# ------------------------------------------------------------ live data ----


def rows_to_bars(rates: Any) -> List[Bar]:
    """MT5 rates (numpy structured array or tuples) → bars."""
    out = []
    for r in rates if rates is not None else []:
        if hasattr(r, "dtype"):
            out.append(Bar(int(r["time"]), float(r["open"]), float(r["high"]), float(r["low"]), float(r["close"]), int(r["tick_volume"])))
        else:
            out.append(Bar(int(r[0]), float(r[1]), float(r[2]), float(r[3]), float(r[4]), int(r[5])))
    return out


def closed_bars(mt5: Any, symbol: str, tf: str, count: int) -> List[Bar]:
    """The last ``count`` CLOSED bars: position 1 skips the bar that is still forming."""
    rates = mt5.copy_rates_from_pos(symbol, getattr(mt5, "TIMEFRAME_" + tf), 1, count)
    if rates is None:
        raise RuntimeError(f"copy_rates_from_pos({symbol}, {tf}) returned None: {mt5.last_error()}")
    return rows_to_bars(rates)


def fmt(ts: int) -> str:
    """Bar time (server clock) as text."""
    return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%d %H:%M")


# --------------------------------------------------------------- orders ----
# Off by default. Remove DRY_RUN only on a DEMO account, after you understand
# what the code does. Real accounts are refused.

DRY_RUN = True
VOLUME = 0.01
MAGIC = 260_510
DEVIATION = 20


def send_order(mt5: Any, symbol: str, side: str, volume: float = VOLUME) -> Optional[Any]:
    """Reverse to ``side``: close this strategy's open positions, then open one."""
    acc = mt5.account_info()
    if acc is None:
        raise RuntimeError("account_info() is None: is MT5 logged in?")
    if acc.trade_mode != mt5.ACCOUNT_TRADE_MODE_DEMO:
        raise RuntimeError("refused: this example only trades on DEMO accounts")
    info, tick = mt5.symbol_info(symbol), mt5.symbol_info_tick(symbol)
    if info is None or tick is None:
        raise RuntimeError(f"{symbol}: no symbol info / tick")
    filling = mt5.ORDER_FILLING_IOC if info.filling_mode & 2 else mt5.ORDER_FILLING_FOK if info.filling_mode & 1 else mt5.ORDER_FILLING_RETURN
    requests = []
    for p in mt5.positions_get(symbol=symbol) or []:
        if p.magic != MAGIC:
            continue
        close_buy = p.type == mt5.POSITION_TYPE_BUY
        requests.append({"action": mt5.TRADE_ACTION_DEAL, "symbol": symbol, "position": p.ticket, "volume": p.volume,
                         "type": mt5.ORDER_TYPE_SELL if close_buy else mt5.ORDER_TYPE_BUY,
                         "price": tick.bid if close_buy else tick.ask})
    buy = side == "BUY"
    requests.append({"action": mt5.TRADE_ACTION_DEAL, "symbol": symbol, "volume": volume,
                     "type": mt5.ORDER_TYPE_BUY if buy else mt5.ORDER_TYPE_SELL, "price": tick.ask if buy else tick.bid})
    result = None
    for req in requests:
        req.update({"deviation": DEVIATION, "magic": MAGIC, "comment": "smt5 supertrend",
                    "type_time": mt5.ORDER_TIME_GTC, "type_filling": filling})
        if DRY_RUN:
            print("  DRY_RUN order_send:", req)
            continue
        result = mt5.order_send(req)
        if result is None or result.retcode != mt5.TRADE_RETCODE_DONE:
            raise RuntimeError(f"order_send failed: {result} {mt5.last_error()}")
    return result


# ----------------------------------------------------------------- live ----


def run_live(mt5: Any, symbol: str, tf: str, params: Params = Params(), warmup: int = 500,
             on_signal: Optional[Callable[[Signal], None]] = None, poll: float = 1.0) -> None:
    """Event loop: warm up on history, then wait for each new closed bar.

    New-bar detection compares the open time of the last closed bar; nothing
    is decided inside a forming bar, so live signals equal backtest signals.
    """
    strat = SupertrendStrategy(params)
    if not mt5.symbol_select(symbol, True):
        raise RuntimeError(f"{symbol}: not available at this broker")
    for bar in closed_bars(mt5, symbol, tf, warmup):        # history: same engine, no orders
        strat.on_bar(bar)
    trend = {1: "UP", -1: "DOWN", 0: "?"}[strat.st.direction]
    print(f"{symbol} {tf}: trend {trend}, line {strat.st.value}, last closed bar {fmt(strat.last_time or 0)}")
    while True:
        try:
            bars = closed_bars(mt5, symbol, tf, 3)            # a few, in case several bars closed meanwhile
        except Exception as exc:                               # bridge restart, network: keep the loop alive
            print("  data error, retrying:", exc)
            time.sleep(5)
            continue
        for bar in bars:
            sig = strat.on_bar(bar)                           # repeated bars are ignored by the engine
            if sig is None:
                continue
            print(f"{fmt(sig.time)}  {sig.side}  close {sig.price}  stop {sig.line:.5f}")
            if on_signal:
                on_signal(sig)
        time.sleep(poll)


# ------------------------------------------------------------------ CLI ----


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("mode", choices=["backtest", "live"])
    ap.add_argument("--symbol", default="EURUSD")
    ap.add_argument("--tf", default="H1", choices=list(TF_SECONDS))
    ap.add_argument("--period", type=int, default=10)
    ap.add_argument("--multiplier", type=float, default=3.0)
    ap.add_argument("--bars", type=int, default=3000, help="history for the backtest")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=8001)
    ap.add_argument("--trade", action="store_true", help="live: call send_order on signals (DRY_RUN still applies)")
    a = ap.parse_args(argv)

    from siliconmetatrader5 import MetaTrader5

    mt5 = MetaTrader5(host=a.host, port=a.port, keepalive=True)
    if not mt5.initialize():
        print("initialize() failed:", mt5.last_error())
        return 1
    params = Params(a.period, a.multiplier)
    try:
        if a.mode == "backtest":
            mt5.symbol_select(a.symbol, True)
            bars = closed_bars(mt5, a.symbol, a.tf, a.bars)
            res = backtest(bars, params)
            s = res.stats
            print(f"{a.symbol} {a.tf}  {len(bars)} closed bars  {fmt(bars[0].time)} → {fmt(bars[-1].time)}")
            for t in res.trades[-10:]:
                print(f"  {t.side:4} {fmt(t.entry_time)} @ {t.entry_price:.5f} → {fmt(t.exit_time or 0)} @ {t.exit_price:.5f}  {t.pnl:+.5f}{'  (open)' if t.open else ''}")
            wr = f"{s['win_rate']:.0%}" if s["win_rate"] is not None else "-"
            pf = f"{s['profit_factor']:.2f}" if s["profit_factor"] is not None else "-"
            print(f"trades {s['trades']}  win rate {wr}  net {s['net']:+.5f}  profit factor {pf}  max drawdown {s['max_drawdown']:.5f}")
        else:
            act = (lambda sig: send_order(mt5, a.symbol, sig.side)) if a.trade else None
            run_live(mt5, a.symbol, a.tf, params, on_signal=act)
    except KeyboardInterrupt:
        pass
    finally:
        mt5.close()          # never shutdown(): other programs share the terminal
    return 0


if __name__ == "__main__":
    sys.exit(main())
