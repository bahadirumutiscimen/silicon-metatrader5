#!/bin/bash
# macOS Silicon Optimized MT5
# Devoloper: bahadirumutiscimen
cd /siliconmt5

# Clear existing display locks to prevent startup errors
rm -rf /tmp/.X100-lock

# Initialize virtual display (Xvfb) and VNC server
export DISPLAY=:100
Xvfb :100 -ac -screen 0 1366x768x24 &
x11vnc -storepasswd $VNC_PWD /siliconmt5/passwd
x11vnc -display :100 -forever -rfbport 5901 -rfbauth /siliconmt5/passwd -ncache 10 &
chmod 600 /siliconmt5/passwd
/siliconmt5/noVNC-master/utils/novnc_proxy --vnc localhost:5901 --listen 6081 &

# Start Openbox Window Manager (Minimalist) - Uncomment to enable
# openbox &

# Check for MT5 installation and install if missing
if [ ! -f "/opt/wineprefix/drive_c/Program Files/MetaTrader 5/terminal64.exe" ]; then
  echo "Installing MetaTrader 5..."
  curl -L -o mt5setup.exe https://download.mql5.com/cdn/web/metaquotes.ltd/mt5/mt5setup.exe
  wine mt5setup.exe /auto
  # Give the installer enough time to complete
  echo "Waiting for MT5 install..."
  sleep 20
  wine taskkill /IM "terminal64.exe" /F || true
  rm mt5setup.exe
fi

# Locate and launch MetaTrader 5
echo "Locating MT5 installation..."
MT5_EXE=$(find /opt/wineprefix/drive_c -name "terminal64.exe" -print -quit)

if [ -z "$MT5_EXE" ]; then
    echo "ERROR: terminal64.exe not found! Installation failed?"
    exit 1
fi

MT5_DIR=$(dirname "$MT5_EXE")
echo "Found MT5 at: $MT5_DIR"

mv "/siliconmt5/mt5cfg.ini" "$MT5_DIR/"
cd "$MT5_DIR"
wine terminal64.exe /portable /config:mt5cfg.ini &
echo "Waiting 15s for MT5 Windows to instantiate..."
sleep 15

# Start the Silicon Bridge (Python Interface)
cd /siliconmt5
wine python -m siliconmetatrader5 --host $MT5_HOST --port 8001 C:/Python/python.exe &
echo "Waiting 30s for MT5 Silicon to instantiate..."
sleep 30

# Process Monitor: Ensure critical services stay running (Anti-Crash)
echo "Starting Watchdog..."

while true
do
  if ! pgrep -f "terminal64.exe" > /dev/null; then
    echo "⚠️ MT5 process not found! Restarting..."
    cd "$MT5_DIR"
    wine terminal64.exe /portable /config:mt5cfg.ini &
    echo "✅ MT5 Restarted."
    sleep 10
  fi
  sleep 5
done