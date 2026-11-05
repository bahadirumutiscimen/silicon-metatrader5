"""SiliconMetaTrader5 Windows display: websockify (noVNC) in front of TightVNC.

Run by the scheduled task SiliconMT5-Display (SYSTEM, at startup), so the
in-app screen is there even while nobody is logged on: the TightVNC service
then shows the Windows logon screen. The task passes websockify's arguments:

  pythonw.exe display.py --log-max-mb <n> --web "<root>\\novnc" 127.0.0.1:<display> 127.0.0.1:<display+1000>

This launcher starts <root>\\python\\python.exe -m websockify with them (no
console window), copies its output into logs\\display.log (kept under
--log-max-mb, the user's SMT5_LOG_MAX_MB: SYSTEM cannot read the user-only
.env, so smt5.ps1 passes the values in the task's arguments), starts it again when it exits (2 s, 4 s,
... 60 s) and records state\\display.json {"launcher", "websockify"} so
smt5.ps1 can tell these processes from the supervisor's. Both addresses must
be 127.0.0.1 (nothing listens on another interface). Standard library only.
"""
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

from supervisor import CREATE_NO_WINDOW, LogPump, log_max_bytes

LOOPBACK = re.compile(r"^127\.0\.0\.1:\d{1,5}$")
STABLE_SECONDS = 60


def websockify_argv(root, args):
    """python.exe -m websockify <args>; the last two arguments (listen and
    target) must be 127.0.0.1:<port>."""
    args = [str(a) for a in args]
    if len(args) < 2 or not all(LOOPBACK.match(a) for a in args[-2:]):
        raise ValueError("display.py: listen and target must be 127.0.0.1:<port>, got %r" % (args[-2:],))
    return [str(Path(root) / "python" / "python.exe"), "-m", "websockify"] + args


def split_args(args):
    """(log limit in bytes, websockify's arguments) from the task's arguments:
    an optional leading `--log-max-mb <n>` (default 10, like SMT5_LOG_MAX_MB)."""
    args = list(args)
    mb = ""
    if len(args) >= 2 and args[0] == "--log-max-mb":
        mb, args = args[1], args[2:]
    return log_max_bytes({"SMT5_LOG_MAX_MB": mb}), args


def backoff(fails):
    return min(2 ** (fails + 1), 60)


def _note(log, msg):
    try:
        with open(log, "a", encoding="utf-8") as fh:
            fh.write("%s display.py: %s\n" % (time.strftime("%Y-%m-%d %H:%M:%S"), msg))
    except OSError:
        pass


def _record(state, launcher, child):
    try:
        state.mkdir(parents=True, exist_ok=True)
        tmp = state / "display.json.tmp"
        tmp.write_text(json.dumps({"launcher": launcher, "websockify": child}), encoding="utf-8")
        os.replace(str(tmp), str(state / "display.json"))
    except OSError:
        pass


def main(argv=None):
    root = Path(__file__).resolve().parents[2]
    args = sys.argv[1:] if argv is None else argv
    log = str(root / "logs" / "display.log")
    (root / "logs").mkdir(parents=True, exist_ok=True)
    max_bytes, args = split_args(args)
    try:
        cmd = websockify_argv(root, args)
    except ValueError as e:
        _note(log, str(e))
        return 2
    fails = 0
    while True:
        started = time.monotonic()
        _note(log, "starting websockify %s -> %s" % (cmd[-2], cmd[-1]))
        try:
            p = subprocess.Popen(cmd, cwd=str(root), stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                 stderr=subprocess.STDOUT, creationflags=CREATE_NO_WINDOW)
        except OSError as e:
            _note(log, "websockify could not start: %s" % e)
            p = None
        if p is not None:
            _record(root / "state", os.getpid(), p.pid)
            LogPump(p.stdout, log, max_bytes).run()  # until websockify exits
            code = p.wait()
            _note(log, "websockify exited (%s)" % code)
        if time.monotonic() - started >= STABLE_SECONDS:
            fails = 0
        time.sleep(backoff(fails))
        fails += 1


if __name__ == "__main__":
    sys.exit(main())
