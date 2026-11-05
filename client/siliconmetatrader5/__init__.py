"""Python client for the SiliconMetaTrader5 bridge: the official ``MetaTrader5`` API,
called over RPyC in a MetaTrader 5 terminal that runs under Wine (macOS Mac
yerel mode, a Docker container on macOS / Linux).

    from siliconmetatrader5 import MetaTrader5
    mt5 = MetaTrader5(host="127.0.0.1", port=8001, keepalive=True)
"""
import datetime as _dt
import threading
import time
from typing import Any, Callable, Dict, Optional

import rpyc
from rpyc.utils.helpers import BgServingThread

from .servertime import bar_time, format_offset, server_now, server_time_offset

# Single source of the version (pyproject.toml reads it from here).
__version__ = "1.3.0"

__all__ = [
    "MetaTrader5",
    "MT5BridgeError",
    "server_now",
    "server_time_offset",
    "format_offset",
    "bar_time",
    "__version__",
]


class MT5BridgeError(RuntimeError):
    """Normalized bridge error with machine-readable metadata."""

    def __init__(self, message: str, *, reason_code: str, operation: str, cause: Exception):
        super().__init__(message)
        self.reason_code = reason_code
        self.operation = operation
        self.cause = cause

class MetaTrader5(object):
    """
    MetaTrader5
    """

    # timeframes
    TIMEFRAME_M1                        = 1
    TIMEFRAME_M2                        = 2
    TIMEFRAME_M3                        = 3
    TIMEFRAME_M4                        = 4
    TIMEFRAME_M5                        = 5
    TIMEFRAME_M6                        = 6
    TIMEFRAME_M10                       = 10
    TIMEFRAME_M12                       = 12
    TIMEFRAME_M15                       = 15
    TIMEFRAME_M20                       = 20
    TIMEFRAME_M30                       = 30
    TIMEFRAME_H1                        = 1  | 0x4000
    TIMEFRAME_H2                        = 2  | 0x4000
    TIMEFRAME_H4                        = 4  | 0x4000
    TIMEFRAME_H3                        = 3  | 0x4000
    TIMEFRAME_H6                        = 6  | 0x4000
    TIMEFRAME_H8                        = 8  | 0x4000
    TIMEFRAME_H12                       = 12 | 0x4000
    TIMEFRAME_D1                        = 24 | 0x4000
    TIMEFRAME_W1                        = 1  | 0x8000
    TIMEFRAME_MN1                       = 1  | 0xC000
    # tick copy flags
    COPY_TICKS_ALL                      = -1
    COPY_TICKS_INFO                     = 1
    COPY_TICKS_TRADE                    = 2
    # tick flags						  
    TICK_FLAG_BID                       = 0x02
    TICK_FLAG_ASK                       = 0x04
    TICK_FLAG_LAST                      = 0x08
    TICK_FLAG_VOLUME                    = 0x10
    TICK_FLAG_BUY                       = 0x20
    TICK_FLAG_SELL                      = 0x40
    # position type, ENUM_POSITION_TYPE
    POSITION_TYPE_BUY                   = 0      # Buy
    POSITION_TYPE_SELL                  = 1      # Sell
    # position reason, ENUM_POSITION_REASON
    POSITION_REASON_CLIENT              = 0      # The position was opened as a result of activation of an order placed from a desktop terminal
    POSITION_REASON_MOBILE              = 1      # The position was opened as a result of activation of an order placed from a mobile application
    POSITION_REASON_WEB                 = 2      # The position was opened as a result of activation of an order placed from the web platform
    POSITION_REASON_EXPERT              = 3      # The position was opened as a result of activation of an order placed from an MQL5 program, i.e. an Expert Advisor or a script
    # order types, ENUM_ORDER_TYPE
    ORDER_TYPE_BUY                      = 0      # Market Buy order
    ORDER_TYPE_SELL                     = 1      # Market Sell order
    ORDER_TYPE_BUY_LIMIT                = 2      # Buy Limit pending order
    ORDER_TYPE_SELL_LIMIT               = 3      # Sell Limit pending order
    ORDER_TYPE_BUY_STOP                 = 4      # Buy Stop pending order
    ORDER_TYPE_SELL_STOP                = 5      # Sell Stop pending order
    ORDER_TYPE_BUY_STOP_LIMIT           = 6      # Upon reaching the order price, a pending Buy Limit order is placed at the StopLimit price
    ORDER_TYPE_SELL_STOP_LIMIT          = 7      # Upon reaching the order price, a pending Sell Limit order is placed at the StopLimit price
    ORDER_TYPE_CLOSE_BY                 = 8      # Order to close a position by an opposite one
    # order state, ENUM_ORDER_STATE
    ORDER_STATE_STARTED                 = 0      # Order checked, but not yet accepted by broker
    ORDER_STATE_PLACED                  = 1      # Order accepted
    ORDER_STATE_CANCELED                = 2      # Order canceled by client
    ORDER_STATE_PARTIAL                 = 3      # Order partially executed
    ORDER_STATE_FILLED                  = 4      # Order fully executed
    ORDER_STATE_REJECTED                = 5      # Order rejected
    ORDER_STATE_EXPIRED                 = 6      # Order expired
    ORDER_STATE_REQUEST_ADD             = 7      # Order is being registered (placing to the trading system)
    ORDER_STATE_REQUEST_MODIFY          = 8      # Order is being modified (changing its parameters)
    ORDER_STATE_REQUEST_CANCEL          = 9      # Order is being deleted (deleting from the trading system)
    # ENUM_ORDER_TYPE_FILLING
    ORDER_FILLING_FOK                   = 0      # Fill Or Kill order
    ORDER_FILLING_IOC                   = 1      # Immediately Or Cancel
    ORDER_FILLING_RETURN                = 2      # Return remaining volume to book
    ORDER_FILLING_BOC                   = 3      # Book Or Cancel order
    # ENUM_ORDER_TYPE_TIME
    ORDER_TIME_GTC                      = 0      # Good till cancel order
    ORDER_TIME_DAY                      = 1      # Good till current trade day order
    ORDER_TIME_SPECIFIED                = 2      # Good till expired order
    ORDER_TIME_SPECIFIED_DAY            = 3      # The order will be effective till 23:59:59 of the specified day. If this time is outside a trading session, the order expires in the nearest trading time.
    # ENUM_ORDER_REASON
    ORDER_REASON_CLIENT                 = 0      # The order was placed from a desktop terminal
    ORDER_REASON_MOBILE                 = 1      # The order was placed from a mobile application
    ORDER_REASON_WEB                    = 2      # The order was placed from a web platform
    ORDER_REASON_EXPERT                 = 3      # The order was placed from an MQL5-program, i.e. by an Expert Advisor or a script
    ORDER_REASON_SL                     = 4      # The order was placed as a result of Stop Loss activation
    ORDER_REASON_TP                     = 5      # The order was placed as a result of Take Profit activation
    ORDER_REASON_SO                     = 6      # The order was placed as a result of the Stop Out event
    # deal types, ENUM_DEAL_TYPE
    DEAL_TYPE_BUY                       = 0      # Buy
    DEAL_TYPE_SELL                      = 1      # Sell
    DEAL_TYPE_BALANCE                   = 2      # Balance
    DEAL_TYPE_CREDIT                    = 3      # Credit
    DEAL_TYPE_CHARGE                    = 4      # Additional charge
    DEAL_TYPE_CORRECTION                = 5      # Correction
    DEAL_TYPE_BONUS                     = 6      # Bonus
    DEAL_TYPE_COMMISSION                = 7      # Additional commission
    DEAL_TYPE_COMMISSION_DAILY          = 8      # Daily commission
    DEAL_TYPE_COMMISSION_MONTHLY        = 9      # Monthly commission
    DEAL_TYPE_COMMISSION_AGENT_DAILY    = 10     # Daily agent commission
    DEAL_TYPE_COMMISSION_AGENT_MONTHLY  = 11     # Monthly agent commission
    DEAL_TYPE_INTEREST                  = 12     # Interest rate
    DEAL_TYPE_BUY_CANCELED              = 13     # Canceled buy deal.
    DEAL_TYPE_SELL_CANCELED             = 14     # Canceled sell deal.
    DEAL_DIVIDEND                       = 15     # Dividend operations
    DEAL_DIVIDEND_FRANKED               = 16     # Franked (non-taxable) dividend operations
    DEAL_TAX                            = 17     # Tax charges
    # ENUM_DEAL_ENTRY
    DEAL_ENTRY_IN                       = 0      # Entry in
    DEAL_ENTRY_OUT                      = 1      # Entry out
    DEAL_ENTRY_INOUT                    = 2      # Reverse
    DEAL_ENTRY_OUT_BY                   = 3      # Close a position by an opposite one
    # ENUM_DEAL_REASON
    DEAL_REASON_CLIENT                  = 0      # The deal was executed as a result of activation of an order placed from a desktop terminal
    DEAL_REASON_MOBILE                  = 1      # The deal was executed as a result of activation of an order placed from a mobile application
    DEAL_REASON_WEB                     = 2      # The deal was executed as a result of activation of an order placed from the web platform
    DEAL_REASON_EXPERT                  = 3      # The deal was executed as a result of activation of an order placed from an MQL5 program, i.e. an Expert Advisor or a script
    DEAL_REASON_SL                      = 4      # The deal was executed as a result of Stop Loss activation
    DEAL_REASON_TP                      = 5      # The deal was executed as a result of Take Profit activation
    DEAL_REASON_SO                      = 6      # The deal was executed as a result of the Stop Out event
    DEAL_REASON_ROLLOVER                = 7      # The deal was executed due to a rollover
    DEAL_REASON_VMARGIN                 = 8      # The deal was executed after charging the variation margin
    DEAL_REASON_SPLIT                   = 9      # The deal was executed after the split (price reduction) of an instrument, which had an open position during split announcement
    # ENUM_TRADE_REQUEST_ACTIONS, Trade Operation Types
    TRADE_ACTION_DEAL                   = 1      # Place a trade order for an immediate execution with the specified parameters (market order)
    TRADE_ACTION_PENDING                = 5      # Place a trade order for the execution under specified conditions (pending order)
    TRADE_ACTION_SLTP                   = 6      # Modify Stop Loss and Take Profit values of an opened position
    TRADE_ACTION_MODIFY                 = 7      # Modify the parameters of the order placed previously
    TRADE_ACTION_REMOVE                 = 8      # Delete the pending order placed previously
    TRADE_ACTION_CLOSE_BY               = 10     # Close a position by an opposite one
    # ENUM_SYMBOL_CHART_MODE
    SYMBOL_CHART_MODE_BID               = 0
    SYMBOL_CHART_MODE_LAST              = 1
    # ENUM_SYMBOL_CALC_MODE
    SYMBOL_CALC_MODE_FOREX              = 0
    SYMBOL_CALC_MODE_FUTURES            = 1
    SYMBOL_CALC_MODE_CFD                = 2
    SYMBOL_CALC_MODE_CFDINDEX           = 3
    SYMBOL_CALC_MODE_CFDLEVERAGE        = 4
    SYMBOL_CALC_MODE_FOREX_NO_LEVERAGE  = 5
    SYMBOL_CALC_MODE_EXCH_STOCKS        = 32
    SYMBOL_CALC_MODE_EXCH_FUTURES       = 33
    SYMBOL_CALC_MODE_EXCH_OPTIONS       = 34
    SYMBOL_CALC_MODE_EXCH_OPTIONS_MARGIN= 36
    SYMBOL_CALC_MODE_EXCH_BONDS         = 37
    SYMBOL_CALC_MODE_EXCH_STOCKS_MOEX   = 38
    SYMBOL_CALC_MODE_EXCH_BONDS_MOEX    = 39
    SYMBOL_CALC_MODE_SERV_COLLATERAL    = 64
    # ENUM_SYMBOL_TRADE_MODE
    SYMBOL_TRADE_MODE_DISABLED          = 0
    SYMBOL_TRADE_MODE_LONGONLY          = 1
    SYMBOL_TRADE_MODE_SHORTONLY         = 2
    SYMBOL_TRADE_MODE_CLOSEONLY         = 3
    SYMBOL_TRADE_MODE_FULL              = 4
    # ENUM_SYMBOL_TRADE_EXECUTION
    SYMBOL_TRADE_EXECUTION_REQUEST      = 0
    SYMBOL_TRADE_EXECUTION_INSTANT      = 1
    SYMBOL_TRADE_EXECUTION_MARKET       = 2
    SYMBOL_TRADE_EXECUTION_EXCHANGE     = 3
    # ENUM_SYMBOL_SWAP_MODE
    SYMBOL_SWAP_MODE_DISABLED           = 0
    SYMBOL_SWAP_MODE_POINTS             = 1
    SYMBOL_SWAP_MODE_CURRENCY_SYMBOL    = 2
    SYMBOL_SWAP_MODE_CURRENCY_MARGIN    = 3
    SYMBOL_SWAP_MODE_CURRENCY_DEPOSIT   = 4
    SYMBOL_SWAP_MODE_INTEREST_CURRENT   = 5
    SYMBOL_SWAP_MODE_INTEREST_OPEN      = 6
    SYMBOL_SWAP_MODE_REOPEN_CURRENT     = 7
    SYMBOL_SWAP_MODE_REOPEN_BID         = 8
    # ENUM_DAY_OF_WEEK
    DAY_OF_WEEK_SUNDAY                  = 0
    DAY_OF_WEEK_MONDAY                  = 1
    DAY_OF_WEEK_TUESDAY                 = 2
    DAY_OF_WEEK_WEDNESDAY               = 3
    DAY_OF_WEEK_THURSDAY                = 4
    DAY_OF_WEEK_FRIDAY                  = 5
    DAY_OF_WEEK_SATURDAY                = 6
    # ENUM_SYMBOL_ORDER_GTC_MODE
    SYMBOL_ORDERS_GTC                   = 0
    SYMBOL_ORDERS_DAILY                 = 1
    SYMBOL_ORDERS_DAILY_NO_STOPS        = 2
    # ENUM_SYMBOL_OPTION_RIGHT
    SYMBOL_OPTION_RIGHT_CALL            = 0
    SYMBOL_OPTION_RIGHT_PUT             = 1
    # ENUM_SYMBOL_OPTION_MODE
    SYMBOL_OPTION_MODE_EUROPEAN         = 0
    SYMBOL_OPTION_MODE_AMERICAN         = 1
    # ENUM_ACCOUNT_TRADE_MODE
    ACCOUNT_TRADE_MODE_DEMO             = 0
    ACCOUNT_TRADE_MODE_CONTEST          = 1
    ACCOUNT_TRADE_MODE_REAL             = 2
    # ENUM_ACCOUNT_STOPOUT_MODE
    ACCOUNT_STOPOUT_MODE_PERCENT        = 0
    ACCOUNT_STOPOUT_MODE_MONEY          = 1
    # ENUM_ACCOUNT_MARGIN_MODE
    ACCOUNT_MARGIN_MODE_RETAIL_NETTING  = 0
    ACCOUNT_MARGIN_MODE_EXCHANGE        = 1
    ACCOUNT_MARGIN_MODE_RETAIL_HEDGING  = 2
    # ENUM_BOOK_TYPE
    BOOK_TYPE_SELL                      = 1
    BOOK_TYPE_BUY                       = 2
    BOOK_TYPE_SELL_MARKET               = 3
    BOOK_TYPE_BUY_MARKET                = 4
    # order send/check return codes
    TRADE_RETCODE_REQUOTE               = 10004
    TRADE_RETCODE_REJECT                = 10006
    TRADE_RETCODE_CANCEL                = 10007
    TRADE_RETCODE_PLACED                = 10008
    TRADE_RETCODE_DONE                  = 10009
    TRADE_RETCODE_DONE_PARTIAL          = 10010
    TRADE_RETCODE_ERROR                 = 10011
    TRADE_RETCODE_TIMEOUT               = 10012
    TRADE_RETCODE_INVALID               = 10013
    TRADE_RETCODE_INVALID_VOLUME        = 10014
    TRADE_RETCODE_INVALID_PRICE         = 10015
    TRADE_RETCODE_INVALID_STOPS         = 10016
    TRADE_RETCODE_TRADE_DISABLED        = 10017
    TRADE_RETCODE_MARKET_CLOSED         = 10018
    TRADE_RETCODE_NO_MONEY              = 10019
    TRADE_RETCODE_PRICE_CHANGED         = 10020
    TRADE_RETCODE_PRICE_OFF             = 10021
    TRADE_RETCODE_INVALID_EXPIRATION    = 10022
    TRADE_RETCODE_ORDER_CHANGED         = 10023
    TRADE_RETCODE_TOO_MANY_REQUESTS     = 10024
    TRADE_RETCODE_NO_CHANGES            = 10025
    TRADE_RETCODE_SERVER_DISABLES_AT    = 10026
    TRADE_RETCODE_CLIENT_DISABLES_AT    = 10027
    TRADE_RETCODE_LOCKED                = 10028
    TRADE_RETCODE_FROZEN                = 10029
    TRADE_RETCODE_INVALID_FILL          = 10030
    TRADE_RETCODE_CONNECTION            = 10031
    TRADE_RETCODE_ONLY_REAL             = 10032
    TRADE_RETCODE_LIMIT_ORDERS          = 10033
    TRADE_RETCODE_LIMIT_VOLUME          = 10034
    TRADE_RETCODE_INVALID_ORDER         = 10035
    TRADE_RETCODE_POSITION_CLOSED       = 10036
    TRADE_RETCODE_INVALID_CLOSE_VOLUME  = 10038
    TRADE_RETCODE_CLOSE_ORDER_EXIST     = 10039
    TRADE_RETCODE_LIMIT_POSITIONS       = 10040
    TRADE_RETCODE_REJECT_CANCEL         = 10041
    TRADE_RETCODE_LONG_ONLY             = 10042
    TRADE_RETCODE_SHORT_ONLY            = 10043
    TRADE_RETCODE_CLOSE_ONLY            = 10044
    TRADE_RETCODE_FIFO_CLOSE            = 10045
    # functio error codes, last_error()
    RES_S_OK                            =1           # generic success
    RES_E_FAIL                          =-1          # generic fail
    RES_E_INVALID_PARAMS                =-2          # invalid arguments/parameters
    RES_E_NO_MEMORY                     =-3          # no memory condition
    RES_E_NOT_FOUND                     =-4          # no history
    RES_E_INVALID_VERSION               =-5          # invalid version
    RES_E_AUTH_FAILED                   =-6          # authorization failed
    RES_E_UNSUPPORTED                   =-7          # unsupported method
    RES_E_AUTO_TRADING_DISABLED         =-8          # auto-trading disabled
    RES_E_INTERNAL_FAIL                 =-10000      # internal IPC general error
    RES_E_INTERNAL_FAIL_SEND            =-10001      # internal IPC send failed
    RES_E_INTERNAL_FAIL_RECEIVE         =-10002      # internal IPC recv failed
    RES_E_INTERNAL_FAIL_INIT            =-10003      # internal IPC initialization fail
    RES_E_INTERNAL_FAIL_CONNECT         =-10004      # internal IPC no ipc
    RES_E_INTERNAL_FAIL_TIMEOUT         =-10005      # internal timeout
    
    def __init__(self, host="localhost", port=18812, keepalive=False, timeout=None):
        """
        Args:

        host: str
            default = localhost
        port: int
            default = 18812
        keepalive: bool
            default = False - enables background thread to keep connection alive
        timeout: Any
            Deprecated and ignored. Kept only for backward compatibility.
        """
        self.__host = host
        self.__port = port
        self.__keepalive = keepalive
        self.__closed = False
        self.__bg_thread = None
        self.__rpc_lock = threading.RLock()
        self.__inflight_call_started_mono: Optional[float] = None
        self.__inflight_call_operation: Optional[str] = None

        # Watchdog state (disabled by default)
        self.__watchdog_stop_event = threading.Event()
        self.__watchdog_thread: Optional[threading.Thread] = None
        self.__watchdog_cfg: Dict[str, Any] = {
            "interval_sec": 5.0,
            "max_miss": 3,
            "probe_timeout_sec": 2.0,
            "stall_timeout_sec": 20.0,
            "auto_reconnect": False,
            "on_unhealthy": None,
        }
        self.__health_lock = threading.Lock()
        self.__health: Dict[str, Any] = {
            "watchdog_running": False,
            "status": "unknown",
            "consecutive_failures": 0,
            "last_probe_ts": None,
            "last_ok_ts": None,
            "last_error": None,
            "last_error_reason": None,
            "inflight_operation": None,
        }

        self.__conn = rpyc.classic.connect(host, port)
        # Timeout feature removed in v1.2.0:
        # keep unbounded sync request timeout to avoid false timeout disconnects
        # on slow/emulated MT5 environments.
        self.__conn._config["sync_request_timeout"] = None

        try:
            self.__mt5 = self.__conn.modules.MetaTrader5
        except Exception:
            # Fallback for older/limited RPyC deployments.
            self.__conn.execute("import MetaTrader5 as mt5")
            self.__mt5 = self.__conn.eval("mt5")
        # Eval-based calls rely on these names existing in the SlaveService namespace.
        self.__conn.execute("import MetaTrader5 as mt5\nimport datetime")

        if self.__keepalive:
            self.__bg_thread = BgServingThread(self.__conn)

    def __del__(self):
        """Destructor - best-effort connection cleanup."""
        try:
            self.close(remote_shutdown=False)
        except Exception:
            pass

    def _ensure_open(self):
        if self.__closed:
            raise RuntimeError("MetaTrader5 connection is already closed")

    def _set_health(self, **updates):
        with self.__health_lock:
            self.__health.update(updates)

    def _get_health_copy(self) -> Dict[str, Any]:
        with self.__health_lock:
            data = dict(self.__health)
        cfg = dict(self.__watchdog_cfg)
        cfg["on_unhealthy"] = bool(cfg.get("on_unhealthy"))
        data["watchdog_config"] = cfg
        return data

    def _classify_probe_error(self, err: Any) -> str:
        if isinstance(err, str):
            return err
        if isinstance(err, MT5BridgeError):
            return err.reason_code
        text = str(err).lower()
        if "timeout" in text:
            return "TIMEOUT"
        if "result expired" in text:
            return "RESULT_EXPIRED"
        if "closed" in text or "broken pipe" in text or "eof" in text:
            return "CONNECTION_CLOSED"
        return "PROBE_FAILED"

    def _inflight_snapshot(self):
        with self.__rpc_lock:
            operation = self.__inflight_call_operation
            started = self.__inflight_call_started_mono
        if started is None:
            return None, operation
        return max(0.0, time.monotonic() - started), operation

    def _probe_connection(self, probe_timeout_sec: float):
        if self.__closed:
            return False, "CLOSED"
        try:
            with self.__rpc_lock:
                previous_timeout = self.__conn._config.get("sync_request_timeout", None)
                self.__conn._config["sync_request_timeout"] = float(probe_timeout_sec)
                try:
                    self.__conn.ping()
                    # Validate remote MT5 module responsiveness (not only socket liveness).
                    self.__mt5.version()
                    return True, None
                finally:
                    self.__conn._config["sync_request_timeout"] = previous_timeout
        except Exception as exc:
            return False, exc

    def _attempt_reconnect(self) -> bool:
        if self.__closed:
            return False
        try:
            with self.__rpc_lock:
                if self.__bg_thread is not None:
                    try:
                        self.__bg_thread.stop()
                    except Exception:
                        pass
                    self.__bg_thread = None

                try:
                    self.__conn.close()
                except Exception:
                    pass

                self.__conn = rpyc.classic.connect(self.__host, self.__port)
                self.__conn._config["sync_request_timeout"] = None
                try:
                    self.__mt5 = self.__conn.modules.MetaTrader5
                except Exception:
                    self.__conn.execute("import MetaTrader5 as mt5")
                    self.__mt5 = self.__conn.eval("mt5")
                self.__conn.execute("import MetaTrader5 as mt5\nimport datetime")

                if self.__keepalive:
                    self.__bg_thread = BgServingThread(self.__conn)
                return True
        except Exception:
            return False

    def _watchdog_loop(self):
        interval = max(0.05, float(self.__watchdog_cfg.get("interval_sec", 5.0)))
        max_miss = max(1, int(self.__watchdog_cfg.get("max_miss", 3)))
        probe_timeout = max(0.05, float(self.__watchdog_cfg.get("probe_timeout_sec", 2.0)))
        stall_timeout = max(0.2, float(self.__watchdog_cfg.get("stall_timeout_sec", 20.0)))
        auto_reconnect = bool(self.__watchdog_cfg.get("auto_reconnect", False))
        callback = self.__watchdog_cfg.get("on_unhealthy")

        while not self.__watchdog_stop_event.wait(interval):
            now_ts = time.time()
            inflight_age, inflight_operation = self._inflight_snapshot()
            self._set_health(last_probe_ts=now_ts, inflight_operation=inflight_operation)

            if inflight_age is not None:
                if inflight_age > stall_timeout:
                    ok = False
                    err = f"CALL_STUCK:{inflight_operation}:{inflight_age:.2f}s"
                else:
                    self._set_health(status="busy")
                    continue
            else:
                ok, err = self._probe_connection(probe_timeout)

            if ok:
                self._set_health(
                    status="healthy",
                    consecutive_failures=0,
                    last_ok_ts=now_ts,
                    last_error=None,
                    last_error_reason=None,
                )
                continue

            state = self._get_health_copy()
            failures = int(state.get("consecutive_failures", 0)) + 1
            reason = self._classify_probe_error(err)
            status = "unhealthy" if failures >= max_miss else "degraded"
            self._set_health(
                status=status,
                consecutive_failures=failures,
                last_error=str(err),
                last_error_reason=reason,
            )

            transitioned_to_unhealthy = state.get("status") != "unhealthy" and status == "unhealthy"
            if transitioned_to_unhealthy and callable(callback):
                try:
                    callback(self._get_health_copy())
                except Exception:
                    pass

            if status == "unhealthy" and auto_reconnect:
                if self._attempt_reconnect():
                    self._set_health(
                        status="recovered",
                        consecutive_failures=0,
                        last_ok_ts=time.time(),
                        last_error=None,
                        last_error_reason=None,
                    )

    def start_watchdog(
        self,
        interval_sec: float = 5.0,
        max_miss: int = 3,
        probe_timeout_sec: float = 2.0,
        stall_timeout_sec: float = 20.0,
        auto_reconnect: bool = False,
        on_unhealthy: Optional[Callable[[Dict[str, Any]], None]] = None,
    ) -> bool:
        """Start watchdog thread to detect frozen/unresponsive MT5 bridge."""
        if self.__closed:
            raise RuntimeError("Cannot start watchdog on closed MetaTrader5 client")
        if self.__watchdog_thread is not None and self.__watchdog_thread.is_alive():
            return False

        self.__watchdog_cfg = {
            "interval_sec": float(interval_sec),
            "max_miss": int(max_miss),
            "probe_timeout_sec": float(probe_timeout_sec),
            "stall_timeout_sec": float(stall_timeout_sec),
            "auto_reconnect": bool(auto_reconnect),
            "on_unhealthy": on_unhealthy,
        }
        self.__watchdog_stop_event.clear()
        self._set_health(
            watchdog_running=True,
            status="starting",
            consecutive_failures=0,
            last_error=None,
            last_error_reason=None,
            inflight_operation=None,
        )
        self.__watchdog_thread = threading.Thread(
            target=self._watchdog_loop,
            name=f"MT5Watchdog:{self.__host}:{self.__port}",
            daemon=True,
        )
        self.__watchdog_thread.start()
        return True

    def stop_watchdog(self, join_timeout: float = 2.0) -> bool:
        """Stop watchdog thread if running."""
        thread = self.__watchdog_thread
        if thread is None:
            self._set_health(watchdog_running=False)
            return False

        self.__watchdog_stop_event.set()
        if threading.current_thread() is not thread:
            thread.join(max(0.0, float(join_timeout)))
        self.__watchdog_thread = None
        self._set_health(watchdog_running=False)
        return True

    def health_status(self) -> Dict[str, Any]:
        """Get current watchdog/connection health snapshot."""
        data = self._get_health_copy()
        data["closed"] = self.__closed
        data["keepalive"] = self.__keepalive
        data["host"] = self.__host
        data["port"] = self.__port
        return data

    def _normalize_datetime(self, value: Any) -> Any:
        if isinstance(value, _dt.datetime):
            # RPyC datetime marshalling can yield invalid-argument results on
            # MT5 date-based APIs. Epoch seconds are bridge-stable.
            try:
                return int(value.timestamp())
            except Exception:
                return value
        return value

    def _wrap_error(self, operation: str, exc: Exception) -> MT5BridgeError:
        text = str(exc).lower()
        if "timeout" in text:
            reason = "TIMEOUT"
        elif "result expired" in text:
            reason = "RESULT_EXPIRED"
        elif "closed" in text or "eof" in text or "broken pipe" in text:
            reason = "CONNECTION_CLOSED"
        else:
            reason = "RPC_ERROR"
        return MT5BridgeError(
            f"{reason}: {operation}: {exc}",
            reason_code=reason,
            operation=operation,
            cause=exc,
        )

    def _remote_call(self, method: str, *args, obtain: bool = False, **kwargs):
        self._ensure_open()
        try:
            with self.__rpc_lock:
                self.__inflight_call_operation = method
                self.__inflight_call_started_mono = time.monotonic()
                fn = getattr(self.__mt5, method)
            result = fn(*args, **kwargs)
            if obtain:
                return rpyc.utils.classic.obtain(result)
            return result
        except Exception as exc:
            raise self._wrap_error(method, exc) from exc
        finally:
            with self.__rpc_lock:
                self.__inflight_call_operation = None
                self.__inflight_call_started_mono = None
    
    def ping(self):
        """Ping the connection to check if it's alive. Returns True if alive."""
        try:
            if self.__closed:
                return False
            with self.__rpc_lock:
                self.__conn.ping()
            return True
        except Exception:
            return False
            
    def initialize(self, *args, **kwargs):
        return self._remote_call("initialize", *args, **kwargs)

    def login(self, *args, **kwargs):
        return self._remote_call("login", *args, **kwargs)

    def shutdown(self, *args, **kwargs):
        """Shutdown remote MT5 terminal (connection stays open for reinitialize)"""
        return self._remote_call("shutdown", *args, **kwargs)
    
    def close(self, remote_shutdown: bool = False):
        """
        Complete cleanup - close everything including background thread and connection.
        Call this when you're done and want to exit the process.
        After calling close(), you cannot use this instance anymore.

        Args:
            remote_shutdown: If True, also calls mt5.shutdown() on remote terminal.
            Default is False to avoid dropping other active clients using same server.
        """
        if self.__closed:
            return

        self.stop_watchdog(join_timeout=1.0)

        if remote_shutdown:
            try:
                self._remote_call("shutdown")
            except Exception:
                pass

        if self.__bg_thread is not None:
            try:
                self.__bg_thread.stop()
            except Exception:
                pass
            self.__bg_thread = None

        try:
            self.__conn.close()
        except Exception:
            pass
        finally:
            self.__closed = True

    def version(self, *args, **kwargs):
        return self._remote_call("version", *args, **kwargs)

    def last_error(self, *args, **kwargs):
        return self._remote_call("last_error", *args, **kwargs)

    def account_info(self, *args, **kwargs):
        return self._remote_eval_call("account_info", *args, **kwargs)

    def terminal_info(self, *args, **kwargs):
        return self._remote_eval_call("terminal_info", *args, **kwargs)

    def symbols_total(self, *args, **kwargs):
        return self._remote_call("symbols_total", *args, **kwargs)

    def symbols_get(self, *args, **kwargs):
        return self._remote_eval_call("symbols_get", *args, **kwargs)

    def symbol_info(self, *args, **kwargs):
        return self._remote_eval_call("symbol_info", *args, **kwargs)

    def symbol_info_tick(self, *args, **kwargs):
        return self._remote_eval_call("symbol_info_tick", *args, **kwargs)

    def symbol_select(self, *args, **kwargs):
        return self._remote_eval_call("symbol_select", *args, **kwargs)

    def market_book_add(self, *args, **kwargs):
        return self._remote_call("market_book_add", *args, **kwargs)

    def market_book_get(self, *args, **kwargs):
        return self._remote_call("market_book_get", *args, **kwargs)

    def market_book_release(self, symbol):
        return self._remote_call("market_book_release", symbol)

    def copy_rates_from(self, symbol, timeframe, date_from, count):
        date_from = self._normalize_datetime(date_from)
        return self._remote_call("copy_rates_from", symbol, timeframe, date_from, count, obtain=True)

    def copy_rates_from_pos(self, symbol, timeframe, start_pos, count):
        return self._remote_call("copy_rates_from_pos", symbol, timeframe, start_pos, count, obtain=True)

    def copy_rates_range(self, symbol, timeframe, date_from, date_to):
        date_from = self._normalize_datetime(date_from)
        date_to = self._normalize_datetime(date_to)
        return self._remote_call("copy_rates_range", symbol, timeframe, date_from, date_to, obtain=True)

    def copy_ticks_from(self, symbol, date_from, count, flags):
        date_from = self._normalize_datetime(date_from)
        return self._remote_call("copy_ticks_from", symbol, date_from, count, flags, obtain=True)

    def copy_ticks_range(self, symbol, date_from, date_to, flags):
        date_from = self._normalize_datetime(date_from)
        date_to = self._normalize_datetime(date_to)
        return self._remote_call("copy_ticks_range", symbol, date_from, date_to, flags, obtain=True)

    def orders_total(self, *args, **kwargs):
        return self._remote_call("orders_total", *args, **kwargs)

    def _remote_eval_call(self, method: str, *args, obtain: bool = False, **kwargs):
        self._ensure_open()
        try:
            with self.__rpc_lock:
                self.__inflight_call_operation = method
                self.__inflight_call_started_mono = time.monotonic()
                code = f"mt5.{method}(*{repr(args)}, **{repr(kwargs)})"
                result = self.__conn.eval(code)
            if obtain:
                return rpyc.utils.classic.obtain(result)
            return result
        except Exception as exc:
            raise self._wrap_error(method, exc) from exc
        finally:
            with self.__rpc_lock:
                self.__inflight_call_operation = None
                self.__inflight_call_started_mono = None

    def orders_get(self, *args, **kwargs):
        return self._remote_eval_call("orders_get", *args, **kwargs)

    def order_calc_margin(self, *args, **kwargs):
        return self._remote_eval_call("order_calc_margin", *args, **kwargs)

    def order_calc_profit(self, *args, **kwargs):
        return self._remote_eval_call("order_calc_profit", *args, **kwargs)

    def order_check(self, *args, **kwargs):
        norm_args = tuple(self._normalize_datetime(arg) for arg in args)
        norm_kwargs = {k: self._normalize_datetime(v) for k, v in kwargs.items()}
        self._ensure_open()
        try:
            with self.__rpc_lock:
                self.__inflight_call_operation = "order_check"
                self.__inflight_call_started_mono = time.monotonic()
                code = f"mt5.order_check(*{repr(norm_args)}, **{repr(norm_kwargs)})"
                return self.__conn.eval(code)
        except Exception as exc:
            raise self._wrap_error("order_check", exc) from exc
        finally:
            with self.__rpc_lock:
                self.__inflight_call_operation = None
                self.__inflight_call_started_mono = None

    def order_send(self, request):
        self._ensure_open()
        try:
            with self.__rpc_lock:
                self.__inflight_call_operation = "order_send"
                self.__inflight_call_started_mono = time.monotonic()
                code = f"mt5.order_send({repr(request)})"
                return self.__conn.eval(code)
        except Exception as exc:
            raise self._wrap_error("order_send", exc) from exc
        finally:
            with self.__rpc_lock:
                self.__inflight_call_operation = None
                self.__inflight_call_started_mono = None

    def positions_total(self, *args, **kwargs):
        return self._remote_call("positions_total", *args, **kwargs)

    def positions_get(self, *args, **kwargs):
        norm_args = tuple(self._normalize_datetime(arg) for arg in args)
        norm_kwargs = {k: self._normalize_datetime(v) for k, v in kwargs.items()}
        return self._remote_eval_call("positions_get", *norm_args, **norm_kwargs)

    def history_orders_total(self, date_from, date_to):
        date_from = self._normalize_datetime(date_from)
        date_to = self._normalize_datetime(date_to)
        return self._remote_call("history_orders_total", date_from, date_to)

    def history_orders_get(self, *args, **kwargs):
        norm_args = tuple(self._normalize_datetime(arg) for arg in args)
        norm_kwargs = {k: self._normalize_datetime(v) for k, v in kwargs.items()}
        return self._remote_eval_call("history_orders_get", *norm_args, **norm_kwargs)

    def history_deals_total(self, date_from, date_to):
        date_from = self._normalize_datetime(date_from)
        date_to = self._normalize_datetime(date_to)
        return self._remote_call("history_deals_total", date_from, date_to)

    def history_deals_get(self, *args, **kwargs):
        norm_args = tuple(self._normalize_datetime(arg) for arg in args)
        norm_kwargs = {k: self._normalize_datetime(v) for k, v in kwargs.items()}
        return self._remote_eval_call("history_deals_get", *norm_args, **norm_kwargs)

    def eval(self, command:str):
        self._ensure_open()
        return self.__conn.eval(command)
    
    def execute(self, command:str):
        self._ensure_open()
        self.__conn.execute(command)
