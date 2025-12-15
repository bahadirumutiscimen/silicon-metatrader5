from siliconmetatrader5 import MetaTrader5
import pandas as pd
import plotly.graph_objects as go
from datetime import datetime
import time

# 1. CONNECTION
print("Connecting to MT5...")
mt5 = MetaTrader5(host="localhost", port=8001)

connected = False
for i in range(5):
    if mt5.initialize():
        print("Connection Successful!")
        connected = True
        break
    else:
        print("Waiting...")
        time.sleep(3)

if not connected:
    print("Could not connect.")
    quit()

# 2. FETCH DATA
symbol = "EURUSD"
timeframe = mt5.TIMEFRAME_M1
count = 100

print(f"Fetching last {count} bars for {symbol}...")
rates = mt5.copy_rates_from_pos(symbol, timeframe, 0, count)

if rates is None or len(rates) == 0:
    print("No data received!")
    mt5.shutdown()
    quit()

# 3. CONVERT TO DATAFRAME
df = pd.DataFrame(rates)
df['time'] = pd.to_datetime(df['time'], unit='s')

print("\n--- SAMPLE DATA ---")
print(df.tail())

# 4. PLOT CHART
fig = go.Figure(data=[go.Candlestick(
    x=df['time'],
    open=df['open'],
    high=df['high'],
    low=df['low'],
    close=df['close']
)])

fig.update_layout(
    title=f"{symbol} - {timeframe} - Last {count} Bars Test",
    xaxis_title="Time",
    yaxis_title="Price",
    template="plotly_dark"
)

# 5. SAVE (as PNG)
import os
script_dir = os.path.dirname(os.path.abspath(__file__))
output_dir = os.path.join(script_dir, "test_charts")
os.makedirs(output_dir, exist_ok=True)
print(f"Saving to directory: {output_dir}")

filename = f"{output_dir}/test_chart_{symbol}_{timeframe}.png"
fig.write_image(filename, width=1920, height=1080)
print(f"\nChart created: {filename}")
print(f"Please check the '{output_dir}' folder.")

mt5.shutdown()
