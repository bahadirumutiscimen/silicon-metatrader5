from datetime import datetime, timedelta
import os
import time

import pandas as pd
import plotly.graph_objects as go
from plotly.subplots import make_subplots

from siliconmetatrader5 import MetaTrader5

from siliconmetatrader5 import server_now  # client >= 1.3.0


def to_df(rates):
    if rates is None or len(rates) == 0:
        return pd.DataFrame()
    df = pd.DataFrame(rates).copy()
    df["time"] = pd.to_datetime(df["time"], unit="s")
    return df


def print_summary(name, df):
    print(f"\n[{name}] rows={len(df)}")
    if df.empty:
        print("No data")
        return
    print("first:", df.iloc[0]["time"], "last:", df.iloc[-1]["time"])
    print(df.tail(3)[["time", "open", "high", "low", "close", "tick_volume", "spread"]])


def add_method_trace(fig, row, title, df):
    if df.empty:
        fig.add_annotation(
            x=0.5,
            y=0.5,
            xref=f"x{row if row > 1 else ''} domain",
            yref=f"y{row if row > 1 else ''} domain",
            text=f"{title}: No data",
            showarrow=False,
            font=dict(size=14, color="red"),
        )
        return

    fig.add_trace(
        go.Candlestick(
            x=df["time"],
            open=df["open"],
            high=df["high"],
            low=df["low"],
            close=df["close"],
            name=title,
            showlegend=False,
        ),
        row=row,
        col=1,
    )


def main():
    print("Connecting to MT5...")
    mt5 = MetaTrader5(host="localhost", port=8001, keepalive=True)

    connected = False
    for i in range(5):
        print(f"Connection attempt {i+1}/5...")
        if mt5.initialize():
            print("Connection successful!")
            connected = True
            break
        time.sleep(3)

    if not connected:
        print("Could not connect.")
        return 1

    symbol = "EURUSD"
    timeframe = mt5.TIMEFRAME_M1
    count = 100

    if not mt5.symbol_select(symbol, True):
        print(f"Symbol {symbol} not found or could not be selected.")
        print("last_error:", mt5.last_error())
        mt5.close()
        return 2

    now = server_now(mt5, symbol)   # broker clock, not this computer's
    range_start = now - timedelta(minutes=max(count * 2, 120))

    print(f"\nFetching 3 methods for {symbol}, timeframe={timeframe}, count={count}...")

    rates_from_pos = mt5.copy_rates_from_pos(symbol, timeframe, 0, count)
    rates_from = mt5.copy_rates_from(symbol, timeframe, now, count)
    rates_range = mt5.copy_rates_range(symbol, timeframe, range_start, now)

    df_pos = to_df(rates_from_pos)
    df_from = to_df(rates_from)
    df_range = to_df(rates_range)

    if not df_range.empty and len(df_range) > count:
        df_range = df_range.tail(count).reset_index(drop=True)

    print_summary("copy_rates_from_pos", df_pos)
    print_summary("copy_rates_from", df_from)
    print_summary("copy_rates_range", df_range)

    if df_pos.empty and df_from.empty and df_range.empty:
        print("All 3 methods returned no data.")
        print("last_error:", mt5.last_error())
        mt5.close()
        return 3

    fig = make_subplots(
        rows=3,
        cols=1,
        shared_xaxes=False,
        vertical_spacing=0.05,
        subplot_titles=(
            "Method 1: copy_rates_from_pos",
            "Method 2: copy_rates_from",
            "Method 3: copy_rates_range",
        ),
    )

    add_method_trace(fig, 1, "copy_rates_from_pos", df_pos)
    add_method_trace(fig, 2, "copy_rates_from", df_from)
    add_method_trace(fig, 3, "copy_rates_range", df_range)

    fig.update_layout(
        title=f"{symbol} | TF={timeframe} | 3 Fetch Methods Comparison",
        template="plotly_dark",
        xaxis_title="Time",
        yaxis_title="Price",
        xaxis2_title="Time",
        yaxis2_title="Price",
        xaxis3_title="Time",
        yaxis3_title="Price",
    )

    script_dir = os.path.dirname(os.path.abspath(__file__))
    output_dir = os.path.join(script_dir, "test_charts")
    os.makedirs(output_dir, exist_ok=True)

    png_path = os.path.join(output_dir, f"test_chart_3methods_{symbol}_{timeframe}.png")
    html_path = os.path.join(output_dir, f"test_chart_3methods_{symbol}_{timeframe}.html")

    fig.write_image(png_path, width=1920, height=1400)
    fig.write_html(html_path)

    print(f"\nChart PNG:  {png_path}")
    print(f"Chart HTML: {html_path}")

    mt5.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
