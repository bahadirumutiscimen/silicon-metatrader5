from siliconmetatrader5 import MetaTrader5
from datetime import datetime
import pandas as pd

print("Connecting to MT5...")
mt5 = MetaTrader5(host="localhost", port=8001)
import time

for i in range(5): # 5 attempts
    print(f"Connection attempt {i+1}/5...")
    if mt5.initialize():
        print("Initialization successful!")
        break
    else:
        print(f"Error: {mt5.last_error()}, waiting 3 seconds...")
        time.sleep(3)
        if i == 4:
            print("All attempts failed. Is MT5 open?")
            quit()

symbol = "EURUSD"
print(f"Checking symbol {symbol}...")
if not mt5.symbol_select(symbol, True):
    print(f"{symbol} not found or could not be added to Market Watch.")
    quit()

print(f"Fetching data for {symbol}...")
rates = mt5.copy_rates_from_pos(symbol, mt5.TIMEFRAME_M5, 0, 10)

if rates is None:
    print("Data retrieval failed (returned None).")
    print("Error code:", mt5.last_error())
else:
    df = pd.DataFrame(rates)
    df['time'] = pd.to_datetime(df['time'], unit='s')
    print("\n--- Last 5 Minutes Data ---")
    print(df[['time', 'open', 'high', 'low', 'close', 'tick_volume', 'spread']])
    print("\nSuccess! Data stream established.")

# 6. Shutdown
mt5.shutdown()
