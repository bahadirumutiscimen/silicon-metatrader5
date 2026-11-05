# siliconmetatrader5

<p align="center"><a href="https://github.com/bahadirumutiscimen/silicon-metatrader5"><img src="https://raw.githubusercontent.com/bahadirumutiscimen/silicon-metatrader5/main/docs/images/banner-en-light.svg" width="760" alt="SiliconMetaTrader5"></a></p>

Python client for **[SiliconMetaTrader5](https://github.com/bahadirumutiscimen/silicon-metatrader5)**:
the official `MetaTrader5` API on macOS (Apple Silicon and Intel), Linux and your own Linux or
Windows servers. MetaTrader 5 runs in the SiliconMetaTrader5 runtime (Wine on macOS and Linux,
natively on a Windows server) and this package talks to it over a local RPyC bridge on 127.0.0.1.

```bash
pip install siliconmetatrader5
```

Set up the runtime first with the desktop app or the `smt5` command line: see the
[project README](https://github.com/bahadirumutiscimen/silicon-metatrader5#readme). The app's
**Bot connection** page shows which host and port to use for wherever your code runs.

## Usage

```python
from datetime import timedelta
import pandas as pd
from siliconmetatrader5 import MetaTrader5, server_now

mt5 = MetaTrader5(host="127.0.0.1", port=8001, keepalive=True)
if not mt5.initialize():
    raise RuntimeError(f"MT5 initialization failed: {mt5.last_error()}")

# live bars
rates = mt5.copy_rates_from_pos("EURUSD", mt5.TIMEFRAME_M15, 0, 150)
print(pd.DataFrame(rates).tail())

# a time range on the broker's clock (not this computer's)
now = server_now(mt5)
rates = mt5.copy_rates_range("EURUSD", mt5.TIMEFRAME_M5, now - timedelta(hours=6), now)

mt5.close()
```

The method names and arguments are those of the official
[MetaTrader5 package](https://www.mql5.com/en/docs/python_metatrader5).

## Broker server time

MT5 stamps bars and ticks with the broker's server wall clock encoded as UTC
(e.g. UTC+2 in winter). Passing `datetime.now()` from your machine therefore
cuts off or shifts the newest bars. Since 1.3.0:

| Function | Returns |
| --- | --- |
| `server_time_offset(mt5, symbol="EURUSD")` | seconds to add to UTC for the broker clock (from a live tick; tries 24/7 symbols when the market is closed), or `None` |
| `server_now(mt5, symbol="EURUSD", offset=None)` | the broker's current time as an aware `datetime`, ready for `copy_rates_from` / `copy_rates_range` |
| `format_offset(offset)` | `"UTC+2"`, `"UTC-4:30"`, `"unknown"` |
| `bar_time(epoch)` | a bar/tick `time` field as an aware `datetime` on the same clock |

## Connection lifecycle

* `close()` disconnects this process; the terminal and other clients keep running.
  `shutdown()` runs MT5's `shutdown()` in the shared bridge process, so every client of
  that bridge loses the terminal connection until it calls `initialize()` again —
  avoid it when several bots share the bridge.
* `keepalive=True` keeps the connection serviced in a background thread (long-running bots).
* `start_watchdog(...)`, `stop_watchdog()`, `health_status()` detect a frozen bridge
  and can reconnect.
* Errors are raised as `MT5BridgeError` with `reason_code` (`TIMEOUT`, `RESULT_EXPIRED`,
  `CONNECTION_CLOSED`, `RPC_ERROR`) and `operation`.

## Changelog

**1.3.0** — broker server-time helpers (`server_now`, `server_time_offset`,
`format_offset`, `bar_time`); `__version__` is correct again (1.2.3 reported
1.2.2); project moved into the SiliconMetaTrader5 repository (`client/`); Python 3.14;
Linux classifier.

**1.2.3** — packaged bridge server layout fixed.

## License

[PolyForm Noncommercial 1.0.0](https://polyformproject.org/licenses/noncommercial/1.0.0): free for personal and other noncommercial use; commercial use needs a separate license from the author.
