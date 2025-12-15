# macOS Silicon Optimized MT5 Docker Image
# Maintainer: Bahadir Umut Iscimen
FROM bahadirumutiscimen/pysiliconwine:3.13.9

WORKDIR /siliconmt5

RUN apt-get update && \
    apt-get upgrade -y && \
    apt-get install -y --no-install-recommends cabextract xvfb x11vnc procps curl unzip python3-websockify python3-numpy openbox tzdata && \
    apt-get clean && \
    curl -L -o noVNC.zip https://github.com/novnc/noVNC/archive/refs/heads/master.zip && unzip noVNC.zip && rm noVNC.zip && \
    rm -rf /var/lib/apt/lists/*

COPY requirements.txt ./
ENV XDG_RUNTIME_DIR=/tmp/runtime-root
RUN mkdir -p /tmp/runtime-root && chmod 700 /tmp/runtime-root && \
    xvfb-run --auto-servernum --server-args="-screen 0 1024x768x24" wine pip install -r requirements.txt

RUN curl -L -o winetricks https://raw.githubusercontent.com/Winetricks/winetricks/master/src/winetricks && \
    chmod +x winetricks && \
    mv winetricks /usr/bin/ && \
    xvfb-run sh -c "winetricks --unattended vcrun2019"

COPY start.sh mt5cfg.ini ./
RUN chmod +x ./start.sh

ENTRYPOINT ["./start.sh"]