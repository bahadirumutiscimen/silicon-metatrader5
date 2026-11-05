#!/bin/bash
# Start the rpyc bridge inside Windows Python. It listens on 0.0.0.0 *inside*
# the container; compose publishes it on 127.0.0.1 of the host only.
#
# The server module is run directly (instead of `python -m siliconmetatrader5
# <python>`, which re-spawns Python through cmd.exe) so there is exactly one
# process to supervise and its command line is easy to match.
set -euo pipefail
SMT5_COMPONENT=bridge
# shellcheck source=common.sh
source /siliconmt5/scripts/common.sh

exec wine "$WIN_PYTHON" -m siliconmetatrader5.server --host 0.0.0.0 -p "$SMT5_BRIDGE_PORT"
