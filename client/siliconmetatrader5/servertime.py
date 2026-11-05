"""Broker server time helpers for the MT5 Python API.

MT5 stores every bar/tick time as the broker's *server wall-clock time*
encoded as if it were UTC (FTMO in winter: UTC+2). The client turns a
``datetime`` into ``int(dt.timestamp())`` — a real UTC epoch — so asking for
``copy_rates_range(..., datetime.now())`` from a Mac in Istanbul (UTC+3)
silently cuts off the newest bars, and "freshness" measured against the local
clock is off by the time-zone difference.

Fix: derive the server offset from a live tick, then build request times on
the server clock::

    from siliconmetatrader5 import server_now

    now = server_now(mt5)                      # aware datetime, server wall clock
    rates = mt5.copy_rates_range("EURUSD", mt5.TIMEFRAME_M5, now - timedelta(hours=6), now)

Works with ``siliconmetatrader5.MetaTrader5`` and with the official
``MetaTrader5`` module. No third-party imports.
"""
from __future__ import annotations

import time
from datetime import datetime, timezone
from typing import Iterable, Optional

__all__ = ["server_time_offset", "server_now", "format_offset", "bar_time", "DEFAULT_FALLBACK_SYMBOLS"]

# Fallbacks tried when the requested symbol has no recent tick (e.g. FX at the
# weekend); crypto CFDs usually trade 24/7.
DEFAULT_FALLBACK_SYMBOLS = ("BTCUSD", "ETHUSD", "BTCUSD.m", "XAUUSD")
OFFSET_STEP = 30 * 60      # broker offsets are whole or half hours
MAX_TICK_AGE = 120         # a tick older than this cannot be trusted


def _offset_from_tick(tick_time: int, now_utc: float, max_tick_age: int) -> Optional[int]:
    raw = tick_time - now_utc
    rounded = int(round(raw / OFFSET_STEP)) * OFFSET_STEP
    if abs(raw - rounded) > max_tick_age or abs(rounded) > 14 * 3600:
        return None  # stale tick (market closed) or nonsense
    return rounded


def server_time_offset(
    mt5,
    symbol: str = "EURUSD",
    fallbacks: Iterable[str] = DEFAULT_FALLBACK_SYMBOLS,
    max_tick_age: int = MAX_TICK_AGE,
) -> Optional[int]:
    """Seconds to add to UTC to get broker server time, or None if unknown.

    Uses the newest tick of ``symbol`` (then the fallbacks). Returns None when
    no candidate has a tick from the last ``max_tick_age`` seconds.
    """
    for sym in (symbol, *fallbacks):
        try:
            if not mt5.symbol_select(sym, True):
                continue
            tick = mt5.symbol_info_tick(sym)
        except Exception:  # noqa: BLE001 — unknown symbol on this broker, bridge hiccup
            continue
        if tick is None or not getattr(tick, "time", 0):
            continue
        offset = _offset_from_tick(int(tick.time), time.time(), max_tick_age)
        if offset is not None:
            return offset
    return None


def format_offset(offset: Optional[int]) -> str:
    """3600 → "UTC+1", -16200 → "UTC-4:30", None → "unknown"."""
    if offset is None:
        return "unknown"
    sign = "+" if offset >= 0 else "-"
    h, m = divmod(abs(offset) // 60, 60)
    return f"UTC{sign}{h}" + (f":{m:02d}" if m else "")


def server_now(mt5, symbol: str = "EURUSD", offset: Optional[int] = None) -> datetime:
    """Current broker server time as an aware datetime (tz labelled UTC).

    The UTC label is deliberate: ``dt.timestamp()`` then yields exactly the
    server-clock epoch MT5 expects, independent of the local time zone.
    Falls back to the newest tick time when the offset cannot be determined.
    """
    if offset is None:
        offset = server_time_offset(mt5, symbol)
    if offset is not None:
        return datetime.fromtimestamp(time.time() + offset, tz=timezone.utc)
    tick = mt5.symbol_info_tick(symbol)
    if tick is not None and getattr(tick, "time", 0):
        return datetime.fromtimestamp(int(tick.time), tz=timezone.utc)
    raise RuntimeError(f"cannot determine broker server time (no ticks for {symbol})")


def bar_time(epoch_seconds: int) -> datetime:
    """MT5 bar/tick 'time' field → aware datetime on the same server clock."""
    return datetime.fromtimestamp(int(epoch_seconds), tz=timezone.utc)
