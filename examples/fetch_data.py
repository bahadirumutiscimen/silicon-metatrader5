import argparse
import time
from datetime import datetime, timedelta

import pandas as pd

from siliconmetatrader5 import MetaTrader5

from siliconmetatrader5 import format_offset, server_now, server_time_offset  # client >= 1.3.0


TF_MINUTES = {"M1": 1, "M5": 5, "M15": 15, "M30": 30, "H1": 60}

TF_MAP = {
    "M1": "TIMEFRAME_M1",
    "M5": "TIMEFRAME_M5",
    "M15": "TIMEFRAME_M15",
    "M30": "TIMEFRAME_M30",
    "H1": "TIMEFRAME_H1",
}


def to_df(rates):
    if rates is None or len(rates) == 0:
        return pd.DataFrame()
    df = pd.DataFrame(rates).copy()
    # Bar times are broker *server* times; keep them as naive server wall-clock.
    df["time"] = pd.to_datetime(df["time"], unit="s")
    return df


def print_summary(name, df):
    print(f"\n[{name}] rows={len(df)}")
    if df.empty:
        print("No data")
        return
    print("first:", df.iloc[0]["time"], "last:", df.iloc[-1]["time"])
    print(df.tail(5)[["time", "open", "high", "low", "close", "tick_volume", "spread"]])


def freshness_minutes(df, now_server_naive):
    """Minutes between the broker's current time and the last bar's open time."""
    if df.empty:
        return None
    return (now_server_naive - df.iloc[-1]["time"]).total_seconds() / 60.0


def main():
    parser = argparse.ArgumentParser(description="Detailed MT5 fetch diagnostics (3 methods)")
    parser.add_argument("--symbol", default="EURUSD")
    parser.add_argument("--tf", default="M5", choices=sorted(TF_MAP.keys()))
    parser.add_argument("--count", type=int, default=100)
    parser.add_argument("--range-hours", type=int, default=6)
    parser.add_argument("--retries", type=int, default=5)
    parser.add_argument("--retry-sleep", type=float, default=3.0)
    args = parser.parse_args()

    print("=== MT5 Fetch Diagnostic (3 Methods) ===")
    print(f"symbol={args.symbol} tf={args.tf} count={args.count} range_hours={args.range_hours}")

    print("\nConnecting to MT5...")
    mt5 = MetaTrader5(host="localhost", port=8001, keepalive=True)

    connected = False
    for i in range(args.retries):
        print(f"Connection attempt {i+1}/{args.retries}...")
        if mt5.initialize():
            print("Initialization successful!")
            connected = True
            break
        try:
            print(f"Error: {mt5.last_error()}, waiting {args.retry_sleep}s...")
        except Exception:
            print(f"Error reading last_error, waiting {args.retry_sleep}s...")
        time.sleep(args.retry_sleep)

    if not connected:
        print("All attempts failed. Is MT5 open?")
        return 1

    timeframe = getattr(mt5, TF_MAP[args.tf])

    print(f"\nChecking symbol {args.symbol}...")
    if not mt5.symbol_select(args.symbol, True):
        print(f"{args.symbol} not found or could not be added to Market Watch.")
        print("last_error:", mt5.last_error())
        mt5.close()
        return 2

    # Requests and freshness use the broker's clock, not this computer's.
    offset = server_time_offset(mt5, args.symbol)
    local_offset = datetime.now().astimezone().utcoffset()
    local_label = format_offset(int(local_offset.total_seconds())) if local_offset is not None else "?"
    print(f"Broker server time: {format_offset(offset)}   (this computer: {local_label})")
    if offset is None:
        print("  No live tick in the last 2 minutes (market closed?): using the last tick time as 'now'.")
    now_server = server_now(mt5, args.symbol, offset)          # aware, server wall clock
    now_naive = now_server.replace(tzinfo=None)
    range_start = now_server - timedelta(hours=args.range_hours)
    print(f"Broker time now: {now_naive:%Y-%m-%d %H:%M:%S}")

    print(f"\nFetching data for {args.symbol}...")

    rates_pos = mt5.copy_rates_from_pos(args.symbol, timeframe, 0, args.count)
    rates_from = mt5.copy_rates_from(args.symbol, timeframe, now_server, args.count)
    rates_range = mt5.copy_rates_range(args.symbol, timeframe, range_start, now_server)

    df_pos = to_df(rates_pos)
    df_from = to_df(rates_from)
    df_range = to_df(rates_range)

    if not df_range.empty and len(df_range) > args.count:
        df_range = df_range.tail(args.count).reset_index(drop=True)

    print_summary("copy_rates_from_pos", df_pos)
    print_summary("copy_rates_from", df_from)
    print_summary("copy_rates_range", df_range)

    f_pos = freshness_minutes(df_pos, now_naive)
    f_from = freshness_minutes(df_from, now_naive)
    f_range = freshness_minutes(df_range, now_naive)

    print("\nFreshness on the broker clock (minutes since last bar opened):")
    print("  from_pos:", "N/A" if f_pos is None else f"{f_pos:.2f}")
    print("  from    :", "N/A" if f_from is None else f"{f_from:.2f}")
    print("  range   :", "N/A" if f_range is None else f"{f_range:.2f}")

    if not df_pos.empty and not df_from.empty:
        delta = (df_pos.iloc[-1]["time"] - df_from.iloc[-1]["time"]).total_seconds() / 60.0
        print(f"\nLast-bar delta (from_pos - from): {delta:.2f} min")

    if not df_pos.empty and not df_range.empty:
        delta = (df_pos.iloc[-1]["time"] - df_range.iloc[-1]["time"]).total_seconds() / 60.0
        print(f"Last-bar delta (from_pos - range): {delta:.2f} min")

    # A live market's last bar opened less than two bar lengths ago.
    limit = 2 * TF_MINUTES[args.tf]
    stale = [n for n, f in (("from_pos", f_pos), ("from", f_from), ("range", f_range)) if f is not None and f > limit]
    if offset is not None and stale:
        print(f"\nWARNING: stale data from {', '.join(stale)} (older than {limit} min). "
              "History may still be syncing; wait a few minutes and retry.")
    elif offset is not None:
        print(f"\nAll methods are fresh (last bar within {limit} min of broker time).")

    all_empty = df_pos.empty and df_from.empty and df_range.empty
    if all_empty:
        print("\nFAILED: All 3 methods returned no data.")
        print("last_error:", mt5.last_error())
        mt5.close()
        return 3

    print("\nSuccess! Data fetch diagnostics completed.")
    print("last_error:", mt5.last_error())

    # Use close() so this test does not intentionally stop shared remote terminal.
    mt5.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
