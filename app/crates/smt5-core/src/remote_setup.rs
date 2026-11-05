//! Server mode setup: what a server lacks (preflight), the apt command that
//! fixes it, the runtime upload and the scripts that install, start, stop
//! and remove it. Everything here is text; `remote` runs it over ssh.

use crate::exec::{Cmd, Runner};
use crate::remote::{shell_quote, Shell};
use crate::runtime::RuntimeDir;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Runtime folder on the server (the app uploads it; `.env` is made there).
const R: &str = "$HOME/siliconmetatrader5";

/// How the runtime runs on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    LinuxNative,
    Docker,
    Windows,
}

impl ServerMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServerMode::LinuxNative => "linux-native",
            ServerMode::Docker => "docker",
            ServerMode::Windows => "windows",
        }
    }
    pub fn parse(s: &str) -> Option<ServerMode> {
        match s {
            "linux-native" => Some(ServerMode::LinuxNative),
            "docker" => Some(ServerMode::Docker),
            "windows" => Some(ServerMode::Windows),
            _ => None,
        }
    }
    /// `SMT5_PROVIDER` in the server's `.env`.
    pub fn provider(&self) -> &'static str {
        match self {
            ServerMode::LinuxNative => "linux-native",
            ServerMode::Docker => "linux-docker",
            ServerMode::Windows => "windows-native",
        }
    }
    /// The shell the server's scripts run in.
    pub fn shell(&self) -> Shell {
        match self {
            ServerMode::Windows => Shell::PowerShell,
            _ => Shell::Bash,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tools {
    #[serde(rename = "Xvfb")]
    pub xvfb: bool,
    pub x11vnc: bool,
    pub websockify: bool,
    pub novnc: bool,
    pub python3_venv: bool,
    pub curl: bool,
    pub xz: bool,
    /// clipboard sync with the desktop app (Server mode)
    pub xclip: bool,
    pub docker: bool,
    pub compose: bool,
}

/// What `runtime/server/preflight.sh` reports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preflight {
    pub arch: String,
    pub os_id: String,
    pub os_pretty: String,
    pub disk_free_mb: u64,
    pub mem_mb: u64,
    pub systemd_user: bool,
    pub linger: bool,
    pub apt: bool,
    pub sudo_nopass: bool,
    pub docker_access: bool,
    pub installed: bool,
    pub version: Option<String>,
    pub tools: Tools,
}

pub fn parse_preflight(json: &str) -> Result<Preflight, String> {
    serde_json::from_str(json.trim()).map_err(|e| format!("preflight: {e}"))
}

/// The preflight script itself (run remotely with `bash -s`).
pub const PREFLIGHT_SH: &str = include_str!("../../../../runtime/server/preflight.sh");

pub const MIN_DISK_MB: u64 = 3072;
pub const MIN_MEM_MB: u64 = 2048;

/// One thing in the way of installing; ids are what the UI translates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub id: &'static str,
    pub blocking: bool,
}

pub fn problems(p: &Preflight, mode: ServerMode) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut add = |id| out.push(Problem { id, blocking: true });
    if p.arch != "x86_64" {
        add("arch");
    }
    if p.disk_free_mb < MIN_DISK_MB {
        add("disk");
    }
    if p.mem_mb < MIN_MEM_MB {
        add("mem");
    }
    let missing = missing_packages(p, mode);
    if !missing.is_empty() {
        add("packages");
    }
    if mode == ServerMode::LinuxNative && !p.linger {
        add("linger");
    }
    if mode == ServerMode::Docker && p.tools.docker && !p.docker_access {
        add("docker_group");
    }
    if !missing.is_empty() && !p.apt {
        add("no_apt");
    }
    out
}

/// What `runtime/windows/preflight.ps1` reports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WinPreflight {
    pub build: u32,
    pub arch: String,
    pub os_caption: String,
    pub admin: bool,
    pub disk_free_mb: u64,
    pub mem_mb: u64,
    pub ps_major: u32,
    pub tar: bool,
    pub network: bool,
    pub pending_reboot: bool,
    pub logon_banner: bool,
    pub foreign_mt5: bool,
    pub installed: bool,
    pub version: Option<String>,
}

/// The Windows preflight script (run remotely through the PowerShell launcher).
pub const PREFLIGHT_PS1: &str = include_str!("../../../../runtime/windows/preflight.ps1");

/// Parses the last line that starts with `{`; warnings or a BOM before it are ignored.
pub fn parse_win_preflight(out: &str) -> Result<WinPreflight, String> {
    let line = out
        .lines()
        .rev()
        .map(|l| l.trim_start_matches('\u{feff}').trim())
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| "preflight: no JSON line in output".to_owned())?;
    serde_json::from_str(line).map_err(|e| format!("preflight: {e}"))
}

/// Windows 10 1809 / Server 2019 is the oldest supported build.
const MIN_WIN_BUILD: u32 = 17763;

pub fn win_problems(p: &WinPreflight) -> Vec<Problem> {
    let mut out = Vec::new();
    let mut add = |id, blocking| out.push(Problem { id, blocking });
    if p.build < MIN_WIN_BUILD {
        add("win_version", true);
    }
    if p.arch != "AMD64" {
        add("arch", true);
    }
    if !p.admin {
        add("not_admin", true);
    }
    if p.disk_free_mb < MIN_DISK_MB {
        add("disk", true);
    }
    if p.mem_mb < MIN_MEM_MB {
        add("mem", true);
    }
    if !p.tar {
        add("no_tar", true);
    }
    if !p.network {
        add("network", true);
    }
    if p.pending_reboot {
        add("pending_reboot", false);
    }
    if p.logon_banner {
        add("logon_banner", false);
    }
    if p.foreign_mt5 {
        add("foreign_mt5", true);
    }
    out
}

/// apt packages for what is missing in `mode`.
pub fn missing_packages(p: &Preflight, mode: ServerMode) -> Vec<&'static str> {
    let t = &p.tools;
    let mut v = Vec::new();
    let mut need = |have: bool, pkgs: &[&'static str]| {
        if !have {
            v.extend_from_slice(pkgs);
        }
    };
    match mode {
        ServerMode::LinuxNative => {
            need(t.xvfb, &["xvfb", "openbox"]);
            need(t.x11vnc, &["x11vnc"]);
            need(t.websockify, &["websockify"]);
            need(t.novnc, &["novnc"]);
            need(t.python3_venv, &["python3-venv"]);
            need(t.curl, &["curl"]);
            need(t.xz, &["xz-utils"]);
            need(t.xclip, &["xclip"]);
        }
        ServerMode::Docker => {
            need(t.docker, &["docker.io"]);
            need(t.compose, &["docker-compose-v2"]);
        }
        ServerMode::Windows => {} // the Windows installer fetches what it needs itself
    }
    v
}

/// The one command the user runs in Terminal (sudo asks there), or None.
pub fn fix_script(p: &Preflight, mode: ServerMode) -> Option<String> {
    let pkgs = missing_packages(p, mode);
    if !pkgs.is_empty() && !p.apt {
        return None;
    }
    let mut parts = Vec::new();
    if !pkgs.is_empty() {
        parts.push(format!("sudo apt-get update && sudo apt-get install -y {}", pkgs.join(" ")));
    }
    if mode == ServerMode::Docker && (!p.tools.docker || !p.docker_access) {
        parts.push("sudo usermod -aG docker \"$USER\"".to_owned());
    }
    if mode == ServerMode::LinuxNative && !p.linger {
        parts.push("sudo loginctl enable-linger \"$USER\"".to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(" && "))
}

fn skip(name: &str) -> bool {
    name == ".env" || name == "__pycache__" || name == ".DS_Store" || name.ends_with(".pyc")
}

fn files(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        if skip(&name) {
            continue;
        }
        let ft = e.file_type()?;
        if ft.is_dir() {
            files(root, &e.path(), out)?;
        } else if ft.is_file() {
            out.push(e.path().strip_prefix(root).unwrap_or(&e.path()).to_path_buf());
        }
    }
    Ok(())
}

/// sha256 (hex) over the runtime's files, sorted, without `.env`/caches:
/// the server's `VERSION`, compared to decide "update needed".
pub fn runtime_digest(rt: &RuntimeDir) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut list = Vec::new();
    files(&rt.root, &rt.root, &mut list).map_err(|e| format!("runtime: {e}"))?;
    list.sort();
    let mut h = Sha256::new();
    for rel in list {
        let bytes = std::fs::read(rt.root.join(&rel)).map_err(|e| format!("{}: {e}", rel.display()))?;
        h.update(rel.to_string_lossy().as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(&bytes);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// tar.gz of the runtime (no `.env`, caches, macOS metadata) at `out`;
/// returns its digest.
pub fn runtime_tarball(rt: &RuntimeDir, out: &Path, runner: &dyn Runner) -> Result<String, String> {
    let parent = rt.root.parent().ok_or("runtime has no parent folder")?;
    let name = rt.root.file_name().ok_or("runtime has no name")?.to_string_lossy().into_owned();
    let o = runner.run(
        &Cmd::new("tar")
            .env("COPYFILE_DISABLE", "1")
            .arg("-czf")
            .arg(out.display().to_string())
            .args(["--no-xattrs", "--exclude=.env", "--exclude=__pycache__", "--exclude=*.pyc", "--exclude=.DS_Store", "-C"])
            .arg(parent.display().to_string())
            .arg(name),
    );
    if !o.ok() {
        return Err(format!("tar: {}", o.stderr.trim()));
    }
    runtime_digest(rt)
}

/// Unpack the tar.gz from stdin as the new runtime, keeping `.env`.
pub fn upload_script(digest: &str) -> String {
    format!(
        r#"set -e
R="{R}"
mkdir -p "$R"
rm -rf "$R/runtime.new" "$R/runtime.old"
mkdir "$R/runtime.new"
tar -xzf - -C "$R/runtime.new" --strip-components=1
if [ -f "$R/runtime/.env" ]; then cp -p "$R/runtime/.env" "$R/runtime.new/.env"; fi
if [ -d "$R/runtime" ]; then mv "$R/runtime" "$R/runtime.old"; fi
mv "$R/runtime.new" "$R/runtime"
rm -rf "$R/runtime.old"
chmod +x "$R"/runtime/macos/*.sh "$R"/runtime/scripts/*.sh "$R"/runtime/server/*.sh 2>/dev/null || true
printf '%s\n' {d} > "$R/VERSION"
echo "runtime uploaded ($HOME/siliconmetatrader5/runtime)""#,
        d = shell_quote(digest)
    )
}

/// Temp file the Windows upload lands in (user profile, removed after unpacking).
pub const WIN_UPLOAD_NAME: &str = "smt5-runtime.tgz";

/// A PowerShell single-quoted string (inner `'` doubled).
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// [`upload_script`] for `mode`; on Windows the tarball was copied to
/// `$HOME\smt5-runtime.tgz` first (scp), not piped through stdin.
pub fn upload_script_for(mode: ServerMode, digest: &str) -> String {
    match mode {
        ServerMode::Windows => format!(
            r#"$ErrorActionPreference='Stop'
try {{
$root = Join-Path $env:LOCALAPPDATA 'siliconmetatrader5'
$tgz = Join-Path $HOME '{WIN_UPLOAD_NAME}'
if (-not (Test-Path -LiteralPath $tgz)) {{ throw 'runtime archive was not uploaded' }}
New-Item -ItemType Directory -Force -Path $root | Out-Null
$cur = Join-Path $root 'runtime'
$new = Join-Path $root 'runtime.new'
$old = Join-Path $root 'runtime.old'
foreach ($d in @($new, $old)) {{ if (Test-Path -LiteralPath $d) {{ Remove-Item -LiteralPath $d -Recurse -Force }} }}
New-Item -ItemType Directory -Path $new | Out-Null
& tar.exe -xzf $tgz -C $new --strip-components=1
if ($LASTEXITCODE -ne 0) {{ throw "tar failed ($LASTEXITCODE)" }}
if (Test-Path -LiteralPath $cur) {{ Move-Item -LiteralPath $cur -Destination $old }}
$envf = Join-Path $old '.env'
if (Test-Path -LiteralPath $envf) {{ Move-Item -LiteralPath $envf -Destination (Join-Path $new '.env') -Force }}
Move-Item -LiteralPath $new -Destination $cur
if (Test-Path -LiteralPath $old) {{ try {{ Remove-Item -LiteralPath $old -Recurse -Force }} catch {{ [Console]::Error.WriteLine("warning: could not remove $old") }} }}
[IO.File]::WriteAllText((Join-Path $root 'VERSION'), ({d} + "`n"), (New-Object Text.UTF8Encoding($false)))
Remove-Item -LiteralPath $tgz -Force
Write-Output "runtime uploaded ($cur)"
}} catch {{
$msg = $_.Exception.Message
try {{
  if ((-not (Test-Path -LiteralPath $cur)) -and (Test-Path -LiteralPath $old)) {{ Move-Item -LiteralPath $old -Destination $cur }}
  $ce = Join-Path $cur '.env'
  if ((Test-Path -LiteralPath $cur) -and (-not (Test-Path -LiteralPath $ce))) {{
    foreach ($d in @($new, $old)) {{ $se = Join-Path $d '.env'; if (Test-Path -LiteralPath $se) {{ Move-Item -LiteralPath $se -Destination $ce -Force; break }} }}
  }}
}} catch {{ $msg = "$msg; rollback failed: $($_.Exception.Message)" }}
[Console]::Error.WriteLine($msg)
exit 1
}}"#,
            d = ps_quote(digest)
        ),
        _ => upload_script(digest),
    }
}

/// First install only: `.env` from the example with secrets made here.
pub fn env_init_script(mode: ServerMode) -> String {
    if mode == ServerMode::Windows {
        return win_env_init_script();
    }
    format!(
        r#"set -e
E="{R}/runtime/.env"
if [ -f "$E" ]; then echo ".env exists (kept)"; exit 0; fi
cp "{R}/runtime/.env.example" "$E"
chmod 600 "$E"
rnd() {{ head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' | cut -c1-"$1"; }}
set_kv() {{ k="${{1%%=*}}"; if grep -q "^$k=" "$E"; then sed -i "s|^$k=.*|$1|" "$E"; else printf '%s\n' "$1" >> "$E"; fi; }}
set_kv "SMT5_WEB_PASSWORD=$(rnd 18)"
set_kv "SMT5_API_TOKEN=$(rnd 48)"
set_kv "SMT5_PROVIDER={p}"
set_kv "SMT5_NATIVE_DISPLAY=novnc"
echo ".env created (secrets stay on this server)""#,
        p = mode.provider()
    )
}

fn win_env_init_script() -> String {
    format!(
        r#"$ErrorActionPreference='Stop'
try {{
$root = Join-Path $env:LOCALAPPDATA 'siliconmetatrader5'
$rt = Join-Path $root 'runtime'
$e = Join-Path $rt '.env'
if (Test-Path -LiteralPath $e) {{ Write-Output '.env exists (kept)'; exit 0 }}
Copy-Item -LiteralPath (Join-Path $rt '.env.example') -Destination $e
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
& icacls.exe $e /inheritance:r /grant:r "*${{sid}}:F" | Out-Null
if ($LASTEXITCODE -ne 0) {{ throw "icacls failed ($LASTEXITCODE)" }}
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
$alpha = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'
$pw = ''
$b = New-Object byte[] 1
while ($pw.Length -lt 8) {{ $rng.GetBytes($b); if ($b[0] -lt 248) {{ $pw += $alpha[$b[0] % 62] }} }}
$tb = New-Object byte[] 24
$rng.GetBytes($tb)
$tok = -join ($tb | ForEach-Object {{ $_.ToString('x2') }})
$kv = [ordered]@{{ SMT5_WEB_PASSWORD = $pw; SMT5_API_TOKEN = $tok; SMT5_PROVIDER = '{p}' }}
$lines = [System.Collections.Generic.List[string]]@([IO.File]::ReadAllText($e) -split "`r?`n")
foreach ($k in $kv.Keys) {{
  $hit = $false
  for ($i = 0; $i -lt $lines.Count; $i++) {{ if ($lines[$i].StartsWith("$k=")) {{ $lines[$i] = "$k=$($kv[$k])"; $hit = $true }} }}
  if (-not $hit) {{ $lines.Add("$k=$($kv[$k])") }}
}}
[IO.File]::WriteAllText($e, (($lines -join "`n").TrimEnd() + "`n"), (New-Object Text.UTF8Encoding($false)))
Write-Output '.env created (secrets stay on this server)'
}} catch {{
[Console]::Error.WriteLine($_.Exception.Message)
exit 1
}}"#,
        p = ServerMode::Windows.provider()
    )
}

/// [`read_env_script`] for `mode`. Windows has no linger: `LINGER=yes`;
/// `INSTALLED=yes` once `smt5.ps1 install` finished for the runtime that is
/// there now (`state\installed` holds the VERSION digest it installed): VERSION
/// and .env alone only mean the runtime was uploaded, and an update whose
/// install step failed leaves the old digest in the marker.
pub fn read_env_script_for(mode: ServerMode) -> String {
    match mode {
        ServerMode::Windows => r#"$ErrorActionPreference='SilentlyContinue'
$root = Join-Path $env:LOCALAPPDATA 'siliconmetatrader5'
Get-Content -LiteralPath (Join-Path $root 'runtime\.env') | Where-Object { $_ -match '^SMT5_(API_TOKEN|WEB_PASSWORD|BRIDGE_PORT|STATUS_PORT|DISPLAY_PORT)=' }
$v = Get-Content -LiteralPath (Join-Path $root 'VERSION') -TotalCount 1
Write-Output "VERSION=$v"
$m = Get-Content -LiteralPath (Join-Path $root 'state\installed') -TotalCount 1
if ($v -and $m -and (([string]$m).Trim() -eq ([string]$v).Trim())) { Write-Output 'INSTALLED=yes' } else { Write-Output 'INSTALLED=no' }
Write-Output 'LINGER=yes'
exit 0"#
            .to_owned(),
        _ => read_env_script().to_owned(),
    }
}

/// Secrets and ports the app needs, plus VERSION and linger (key=value lines).
pub fn read_env_script() -> &'static str {
    r#"R="$HOME/siliconmetatrader5"
grep -E '^SMT5_(API_TOKEN|WEB_PASSWORD|BRIDGE_PORT|STATUS_PORT|DISPLAY_PORT)=' "$R/runtime/.env" 2>/dev/null
printf 'VERSION=%s\n' "$(head -n1 "$R/VERSION" 2>/dev/null)"
printf 'LINGER=%s\n' "$(loginctl show-user "${USER:-$(id -un)}" -p Linger --value 2>/dev/null || echo no)"
true"#
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Install,
    Start,
    Stop,
    Restart,
    /// MT5 data only (Wine prefix / Docker volumes); runtime and .env stay
    Reset,
    Uninstall,
}

/// Restart a Windows server in 5 s (planned, forced; prints `rebooting`).
/// Self-contained: the precheck offers it before the runtime is uploaded.
pub fn win_reboot_script() -> String {
    r#"$ErrorActionPreference = 'Stop'
$p = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  [Console]::Error.WriteLine('not_admin: restarting Windows needs an administrator (elevated) SSH session')
  exit 1
}
$ErrorActionPreference = 'Continue'
$out = (& shutdown.exe @('/r', '/f', '/t', '5', '/d', 'p:4:1', '/c', 'SiliconMetaTrader5') 2>&1 | ForEach-Object { [string]$_ }) -join ' '
$code = $LASTEXITCODE
# 1190: a shutdown is already scheduled
if ($code -ne 0 -and $code -ne 1190) {
  [Console]::Error.WriteLine("shutdown.exe failed ($code): $out")
  exit 1
}
[Console]::Out.WriteLine('rebooting')
exit 0"#
        .to_owned()
}

/// `smt5.ps1 <action>` on a Windows server (`install|start|stop|restart|reset|uninstall`
/// or `autologon-status|session|reboot`).
pub fn win_action_script(action: &str) -> String {
    format!(
        "$ErrorActionPreference='Stop'\n& \"$env:LOCALAPPDATA\\siliconmetatrader5\\runtime\\windows\\smt5.ps1\" {action}\nexit $LASTEXITCODE"
    )
}

pub fn lifecycle_script(mode: ServerMode, a: Action) -> String {
    match mode {
        ServerMode::Windows => win_action_script(match a {
            Action::Install => "install",
            Action::Start => "start",
            Action::Stop => "stop",
            Action::Restart => "restart",
            Action::Reset => "reset",
            Action::Uninstall => "uninstall",
        }),
        ServerMode::LinuxNative => {
            let sub = match a {
                Action::Install => "install",
                Action::Start => "start",
                Action::Stop => "stop",
                Action::Restart => "restart",
                Action::Reset => "reset",
                Action::Uninstall => "uninstall",
            };
            let mut s = format!(
                r#"set -e
SMT5_ENV_FILE="{R}/runtime/.env" SMT5_NATIVE_DISPLAY=novnc bash "{R}/runtime/macos/native.sh" {sub}"#
            );
            if a == Action::Uninstall {
                // files go only when the teardown worked (set -e stops before)
                s += &format!("\nrm -rf \"{R}\"\necho removed");
            }
            s
        }
        ServerMode::Docker => {
            let sub = match a {
                Action::Install => "up -d --build",
                Action::Start => "up -d",
                Action::Stop => "stop",
                Action::Restart => "restart",
                Action::Reset => "down -v",
                Action::Uninstall => "down -v",
            };
            let mut s = format!(
                r#"set -e
docker compose -f "{R}/runtime/compose.yaml" --env-file "{R}/runtime/.env" {sub}"#
            );
            if a == Action::Uninstall {
                s += &format!("\nrm -rf \"{R}\"\necho removed");
            }
            s
        }
    }
}

/// Marker line of [`stats_script`]'s output.
const STATS_TAG: &str = "SMT5STATS";

/// CPU and memory of the runtime on a server, printed as one line.
///
/// Native: the SSH user's MetaTrader5 processes (our folder, wineserver and
/// the Windows processes Wine names after their Windows command line),
/// sampled twice one second apart from `/proc`, so CPU is the current load
/// (Linux `ps %cpu` is a lifetime average). Docker: `docker stats`.
pub fn stats_script(mode: ServerMode) -> String {
    match mode {
        ServerMode::LinuxNative => format!(
            r#"base="$HOME/.local/share/siliconmt5"
pat="$base|"'terminal64\.exe|python313|^c:\\windows\\|wineserver'
pids=$(pgrep -u "$(id -u)" -f -i -- "$pat")
sample() {{ t=0; r=0; for p in $pids; do [ "$p" = "$$" ] && continue; read -r s 2>/dev/null < "/proc/$p/stat" || continue; s=${{s##*) }}; set -- $s; t=$((t + ${{12}} + ${{13}})); r=$((r + ${{22}})); done; echo "$t $r"; }}
a=$(sample); sleep 1; b=$(sample)
echo "{STATS_TAG} native $(getconf CLK_TCK) $(getconf PAGESIZE) $(awk '/^MemTotal:/{{print $2}}' /proc/meminfo) $a $b""#
        ),
        ServerMode::Windows => win_action_script("stats"),
        ServerMode::Docker => format!(
            r#"echo "{STATS_TAG} docker $(docker stats --no-stream --format '{{{{.CPUPerc}}}}|{{{{.MemUsage}}}}|{{{{.MemPerc}}}}' siliconmt5 2>/dev/null)""#
        ),
    }
}

/// [`stats_script`]'s line → stats (None while nothing runs or on bad output).
pub fn parse_stats(out: &str) -> Option<crate::docker::ContainerStats> {
    let line = out.lines().rev().map(|l| l.trim_start_matches('\u{feff}').trim()).find(|l| l.starts_with(STATS_TAG))?;
    let mut it = line.split_whitespace().skip(1);
    match it.next()? {
        "docker" => crate::docker::parse_stats(&it.collect::<Vec<_>>().join(" ")),
        "native" => {
            let n: Vec<u64> = it.map(|x| x.parse().ok()).collect::<Option<_>>()?;
            let [hz, page, mem_kb, t0, _r0, t1, r1] = n[..] else { return None };
            if hz == 0 || r1 == 0 {
                return None; // no runtime process
            }
            let used = r1 * page;
            let total = mem_kb * 1024;
            Some(crate::docker::ContainerStats {
                cpu_percent: t1.saturating_sub(t0) as f64 * 100.0 / hz as f64,
                mem_used_bytes: used,
                mem_limit_bytes: total,
                mem_percent: if total > 0 { used as f64 * 100.0 / total as f64 } else { 0.0 },
            })
        }
        "win" => {
            let cpu: f64 = it.next()?.parse().ok()?;
            let used: u64 = it.next()?.parse().ok()?;
            let total: u64 = it.next()?.parse().ok()?;
            if used == 0 {
                return None; // no runtime process
            }
            Some(crate::docker::ContainerStats {
                cpu_percent: cpu,
                mem_used_bytes: used,
                mem_limit_bytes: total,
                mem_percent: if total > 0 { used as f64 * 100.0 / total as f64 } else { 0.0 },
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::SystemRunner;

    fn pf(over: &str) -> Preflight {
        let base = r#"{"arch":"x86_64","os_id":"ubuntu","os_pretty":"Ubuntu 26.04","disk_free_mb":190000,"mem_mb":61000,
            "systemd_user":true,"linger":true,"apt":true,"sudo_nopass":false,"docker_access":false,"installed":false,"version":null,
            "tools":{"Xvfb":true,"x11vnc":true,"websockify":true,"novnc":true,"python3_venv":true,"curl":true,"xz":true,"xclip":true,"docker":false,"compose":false}}"#;
        let mut v: serde_json::Value = serde_json::from_str(base).unwrap();
        let o: serde_json::Value = serde_json::from_str(over).unwrap();
        for (k, val) in o.as_object().unwrap() {
            if k == "tools" {
                for (t, b) in val.as_object().unwrap() {
                    v["tools"][t] = b.clone();
                }
            } else {
                v[k] = val.clone();
            }
        }
        parse_preflight(&v.to_string()).unwrap()
    }
    fn ids(p: &[Problem]) -> Vec<&'static str> {
        p.iter().map(|x| x.id).collect()
    }

    #[test]
    fn win_preflight_problems_in_order() {
        let bad = WinPreflight {
            build: 14393,
            arch: "ARM64".into(),
            admin: false,
            disk_free_mb: 100,
            mem_mb: 100,
            tar: false,
            network: false,
            pending_reboot: true,
            logon_banner: true,
            foreign_mt5: true,
            ..Default::default()
        };
        assert_eq!(
            ids(&win_problems(&bad)),
            vec![
                "win_version", "arch", "not_admin", "disk", "mem", "no_tar", "network", "pending_reboot",
                "logon_banner", "foreign_mt5"
            ]
        );
        let good = WinPreflight {
            build: 20348,
            arch: "AMD64".into(),
            admin: true,
            disk_free_mb: 190000,
            mem_mb: 8000,
            tar: true,
            network: true,
            ..Default::default()
        };
        assert!(win_problems(&good).is_empty());
    }

    #[test]
    fn parse_win_preflight_skips_noise() {
        let out = "\u{feff}WARNING: x\n{\"build\":20348,\"arch\":\"AMD64\",\"os_caption\":\"Microsoft Windows Server 2022 Datacenter — Türkçe\"}\n";
        let p = parse_win_preflight(out).unwrap();
        assert_eq!(p.build, 20348);
        assert_eq!(p.arch, "AMD64");
        assert_eq!(p.os_caption, "Microsoft Windows Server 2022 Datacenter — Türkçe");
        assert!(parse_win_preflight("no json here").is_err());
    }

    #[test]
    fn preflight_ps1_emits_all_fields() {
        for f in [
            "build", "arch", "os_caption", "admin", "disk_free_mb", "mem_mb", "ps_major", "tar", "network",
            "pending_reboot", "logon_banner", "foreign_mt5", "installed", "version",
        ] {
            assert!(PREFLIGHT_PS1.contains(f), "preflight.ps1 lacks {f}");
        }
        assert!(PREFLIGHT_PS1.contains("ConvertTo-Json -Compress"));
    }

    #[test]
    fn preflight_problems() {
        assert!(ids(&problems(&pf(r#"{"arch":"aarch64"}"#), ServerMode::LinuxNative)).contains(&"arch"));
        assert!(ids(&problems(&pf(r#"{"disk_free_mb":2000}"#), ServerMode::LinuxNative)).contains(&"disk"));
        assert!(ids(&problems(&pf(r#"{"mem_mb":1500}"#), ServerMode::LinuxNative)).contains(&"mem"));

        let native = pf(r#"{"linger":false,"tools":{"Xvfb":false}}"#);
        assert_eq!(ids(&problems(&native, ServerMode::LinuxNative)), vec!["packages", "linger"]);
        let fix = fix_script(&native, ServerMode::LinuxNative).unwrap();
        assert!(fix.contains("apt-get install -y xvfb openbox") && fix.contains("enable-linger"), "{fix}");
        assert!(problems(&native, ServerMode::LinuxNative).iter().all(|p| p.blocking));

        let docker = pf(r#"{"linger":false,"tools":{"docker":true,"compose":true}}"#);
        assert_eq!(ids(&problems(&docker, ServerMode::Docker)), vec!["docker_group"], "linger not needed for docker");
        let dfix = fix_script(&docker, ServerMode::Docker).unwrap();
        assert!(dfix.contains("usermod -aG docker") && !dfix.contains("apt-get") && !dfix.contains("linger"), "{dfix}");

        let nodocker = pf("{}");
        assert_eq!(missing_packages(&nodocker, ServerMode::Docker), vec!["docker.io", "docker-compose-v2"]);
        assert!(fix_script(&nodocker, ServerMode::Docker).unwrap().contains("usermod -aG docker"), "new docker install also joins the group");

        let ok = pf("{}");
        assert!(problems(&ok, ServerMode::LinuxNative).is_empty());
        assert_eq!(fix_script(&ok, ServerMode::LinuxNative), None);

        let noapt = pf(r#"{"apt":false,"tools":{"x11vnc":false}}"#);
        assert_eq!(ids(&problems(&noapt, ServerMode::LinuxNative)), vec!["packages", "no_apt"]);
        assert_eq!(fix_script(&noapt, ServerMode::LinuxNative), None);
        assert!(parse_preflight("not json").is_err());
        assert_eq!(ServerMode::parse("docker"), Some(ServerMode::Docker));
        assert_eq!(ServerMode::LinuxNative.as_str(), "linux-native");
    }

    #[test]
    fn tarball_is_deterministic() {
        let repo_rt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../runtime");
        let tmp = std::env::temp_dir().join(format!("smt5-tar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let copy = tmp.join("runtime");
        let st = std::process::Command::new("cp").arg("-R").arg(&repo_rt).arg(&copy).status().unwrap();
        assert!(st.success());
        let rt = RuntimeDir::new(&copy);
        let out = tmp.join("a.tgz");
        let d1 = runtime_tarball(&rt, &out, &SystemRunner).unwrap();
        assert!(out.metadata().unwrap().len() > 1000);
        let d2 = runtime_tarball(&rt, &tmp.join("b.tgz"), &SystemRunner).unwrap();
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 64);
        std::fs::write(copy.join(".env"), "SMT5_API_TOKEN=secret\n").unwrap();
        std::fs::create_dir_all(copy.join("status/__pycache__")).unwrap();
        std::fs::write(copy.join("status/__pycache__/x.pyc"), "x").unwrap();
        assert_eq!(runtime_tarball(&rt, &tmp.join("c.tgz"), &SystemRunner).unwrap(), d1, ".env/pyc do not count");
        let list = std::process::Command::new("tar").arg("-tzf").arg(tmp.join("c.tgz")).output().unwrap();
        let names = String::from_utf8_lossy(&list.stdout);
        assert!(!names.contains(".env\n") && !names.contains(".pyc"), "{names}");
        assert!(names.contains("runtime/macos/native.sh"));
        let mut c = std::fs::read_to_string(copy.join("compose.yaml")).unwrap();
        c.push('\n');
        std::fs::write(copy.join("compose.yaml"), c).unwrap();
        assert_ne!(runtime_tarball(&rt, &tmp.join("d.tgz"), &SystemRunner).unwrap(), d1);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scripts_quote_paths() {
        let start = lifecycle_script(ServerMode::LinuxNative, Action::Start);
        assert!(start.contains("native.sh") && start.contains(" start") && start.contains("SMT5_ENV_FILE="), "{start}");
        assert!(lifecycle_script(ServerMode::LinuxNative, Action::Install).contains(" install"));
        assert!(lifecycle_script(ServerMode::Docker, Action::Install).contains("up -d --build"));
        let un = lifecycle_script(ServerMode::Docker, Action::Uninstall);
        assert!(un.contains("down -v") && un.contains("rm -rf \"$HOME/siliconmetatrader5\""), "{un}");
        let init = env_init_script(ServerMode::Docker);
        assert!(init.contains("[ -f") && init.contains("/dev/urandom") && init.contains("SMT5_PROVIDER=linux-docker") && init.contains("SMT5_NATIVE_DISPLAY=novnc"));
        let up = upload_script("d1g3st");
        assert!(up.contains("runtime.new") && up.contains("d1g3st") && up.contains(".env"));
        assert!(read_env_script().contains("LINGER="));
        // MT5 data reset on a server: prefix (native) / volumes (docker), runtime stays
        let rn = lifecycle_script(ServerMode::LinuxNative, Action::Reset);
        assert!(rn.contains("native.sh") && rn.contains(" reset") && !rn.contains("rm -rf"), "{rn}");
        let rd = lifecycle_script(ServerMode::Docker, Action::Reset);
        assert!(rd.contains("down -v") && !rd.contains("rm -rf"), "{rd}");
    }

    #[test]
    fn upload_then_env_init_on_a_fake_home() {
        let repo_rt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../runtime");
        let tmp = std::env::temp_dir().join(format!("smt5-up-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("home")).unwrap();
        let tgz = tmp.join("rt.tgz");
        let digest = runtime_tarball(&RuntimeDir::new(&repo_rt), &tgz, &SystemRunner).unwrap();
        let run = |script: &str| {
            let o = SystemRunner.run(&Cmd::new("bash").args(["-c", script]).env("HOME", &tmp.join("home").display().to_string()).stdin_file(&tgz));
            assert!(o.ok(), "{script}\n{}", o.stderr);
        };
        run(&upload_script(&digest));
        let r = tmp.join("home/siliconmetatrader5");
        assert!(r.join("runtime/macos/native.sh").exists());
        assert_eq!(std::fs::read_to_string(r.join("VERSION")).unwrap().trim(), digest);
        run(&env_init_script(ServerMode::LinuxNative));
        let env1 = std::fs::read_to_string(r.join("runtime/.env")).unwrap();
        assert!(env1.contains("SMT5_PROVIDER=linux-native") && env1.contains("SMT5_NATIVE_DISPLAY=novnc"));
        let token = env1.lines().find_map(|l| l.strip_prefix("SMT5_API_TOKEN=")).unwrap();
        assert_eq!(token.len(), 48);
        run(&upload_script(&digest)); // an update keeps .env
        run(&env_init_script(ServerMode::LinuxNative)); // and init does not overwrite it
        assert_eq!(std::fs::read_to_string(r.join("runtime/.env")).unwrap(), env1);
        assert!(!r.join("runtime.new").exists() && !r.join("runtime.old").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn uninstall_keeps_files_when_teardown_fails() {
        // Final review I7: a failed `compose down` must not delete the runtime folder
        let tmp = std::env::temp_dir().join(format!("smt5-un-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("bin")).unwrap();
        std::fs::create_dir_all(tmp.join("home/siliconmetatrader5/runtime")).unwrap();
        std::fs::write(tmp.join("bin/docker"), "#!/bin/sh\nexit 1\n").unwrap();
        let _ = std::process::Command::new("chmod").args(["+x"]).arg(tmp.join("bin/docker")).status();
        let path = format!("{}:/usr/bin:/bin", tmp.join("bin").display());
        let o = std::process::Command::new("bash")
            .args(["-c", &lifecycle_script(ServerMode::Docker, Action::Uninstall)])
            .env("HOME", tmp.join("home"))
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(!o.status.success(), "the job must fail");
        assert!(tmp.join("home/siliconmetatrader5/runtime").exists(), "files kept for a retry");
        std::fs::write(tmp.join("bin/docker"), "#!/bin/sh\nexit 0\n").unwrap();
        let ok = std::process::Command::new("bash")
            .args(["-c", &lifecycle_script(ServerMode::Docker, Action::Uninstall)])
            .env("HOME", tmp.join("home"))
            .env("PATH", &path)
            .status()
            .unwrap();
        assert!(ok.success() && !tmp.join("home/siliconmetatrader5").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn scripts_are_valid_bash() {
        let mut all = vec![env_init_script(ServerMode::LinuxNative), upload_script("x"), read_env_script().to_owned()];
        for m in [ServerMode::LinuxNative, ServerMode::Docker] {
            for a in [Action::Install, Action::Start, Action::Stop, Action::Restart, Action::Reset, Action::Uninstall] {
                all.push(lifecycle_script(m, a));
            }
        }
        for s in all {
            let o = std::process::Command::new("bash").args(["-n", "-c", &s]).output().unwrap();
            assert!(o.status.success(), "{s}\n{}", String::from_utf8_lossy(&o.stderr));
        }
    }
    #[test]
    fn stats_native_is_cpu_delta_and_rss() {
        // 100 Hz, 4 KiB pages, 8 GiB; 150 ticks in one second = 150 % (1.5 cores)
        let st = parse_stats("motd\nSMT5STATS native 100 4096 8388608 1000 250000 1150 262144\n").unwrap();
        assert!((st.cpu_percent - 150.0).abs() < 1e-9);
        assert_eq!(st.mem_used_bytes, 262144 * 4096);
        assert_eq!(st.mem_limit_bytes, 8 * 1024 * 1024 * 1024);
        assert!((st.mem_percent - 12.5).abs() < 1e-9);
        assert!(parse_stats("SMT5STATS native 100 4096 8388608 0 0 0 0").is_none(), "nothing running");
        assert!(parse_stats("SMT5STATS native 100 4096").is_none());
        assert!(parse_stats("nothing").is_none());
    }

    #[test]
    fn stats_docker_reuses_docker_stats() {
        let st = parse_stats("SMT5STATS docker 42.10%|5.1GiB / 7.7GiB|66.2%").unwrap();
        assert!((st.cpu_percent - 42.1).abs() < 1e-9);
        assert!(parse_stats("SMT5STATS docker ").is_none(), "container not running");
    }

    #[test]
    fn stats_script_is_valid_bash_and_reads_proc() {
        for m in [ServerMode::LinuxNative, ServerMode::Docker] {
            let sc = stats_script(m);
            let o = std::process::Command::new("bash").args(["-n", "-c", &sc]).output().unwrap();
            assert!(o.status.success(), "{m:?}: {}", String::from_utf8_lossy(&o.stderr));
        }
        // run the native one for real: a sleeping process named like MT5
        if std::path::Path::new("/proc/self/stat").exists() {
            // as sshd runs it: the remote command line goes through the login shell
            let line = format!("bash -lc {}", shell_quote(&stats_script(ServerMode::LinuxNative)));
            let o = std::process::Command::new("sh").arg("-c").arg(line).output().unwrap();
            let out = String::from_utf8_lossy(&o.stdout);
            assert!(out.contains("SMT5STATS native "), "{out}");
        }
    }

    #[test]
    fn windows_mode_roundtrip() {
        assert_eq!(ServerMode::parse("windows"), Some(ServerMode::Windows));
        assert_eq!(ServerMode::Windows.as_str(), "windows");
        assert_eq!(ServerMode::Windows.provider(), "windows-native");
        assert_eq!(ServerMode::Windows.shell(), Shell::PowerShell);
        assert_eq!(ServerMode::Docker.shell(), Shell::Bash);
        assert_eq!(ServerMode::LinuxNative.shell(), Shell::Bash);
    }

    #[test]
    fn windows_lifecycle_calls_smt5_ps1() {
        for (a, sub) in [
            (Action::Install, "install"),
            (Action::Start, "start"),
            (Action::Stop, "stop"),
            (Action::Restart, "restart"),
            (Action::Reset, "reset"),
            (Action::Uninstall, "uninstall"),
        ] {
            let s = lifecycle_script(ServerMode::Windows, a);
            assert!(s.contains(&format!("smt5.ps1\" {sub}")) && s.contains("$env:LOCALAPPDATA"), "{s}");
            assert!(!s.contains("bash") && s.contains("exit $LASTEXITCODE"), "{s}");
        }
        for sub in ["autologon-status", "session", "reboot"] {
            let s = win_action_script(sub);
            assert!(s.contains(&format!("smt5.ps1\" {sub}")) && s.contains("$env:LOCALAPPDATA"), "{s}");
        }
        let st = stats_script(ServerMode::Windows);
        assert!(st.contains("smt5.ps1\" stats"), "{st}");
    }

    #[test]
    fn windows_upload_keeps_env_and_writes_version() {
        let s = upload_script_for(ServerMode::Windows, "d1'g");
        for needle in ["smt5-runtime.tgz", ".env", "VERSION", "--strip-components=1", "$ErrorActionPreference='Stop'", "'d1''g'"] {
            assert!(s.contains(needle), "missing {needle}\n{s}");
        }
        assert!(!s.contains("bash"));
        let (m_old, m_env) = (s.find("Move-Item -LiteralPath $cur -Destination $old").unwrap(), s.find("Move-Item -LiteralPath $envf").unwrap());
        assert!(m_old < m_env, ".env moves only after cur -> old succeeded");
        assert!(s.contains("$envf = Join-Path $old '.env'") && s.contains("(-not (Test-Path -LiteralPath $ce))") && s.contains("foreach ($d in @($new, $old))"), ".env restore in catch");
        assert!(s.contains("-not (Test-Path -LiteralPath $cur)) -and (Test-Path -LiteralPath $old)") && s.contains("Move-Item -LiteralPath $old -Destination $cur"), "rollback");
        assert!(s.contains("[Console]::Error.WriteLine($msg)\nexit 1") && !s.contains("Write-Error"));
        assert_eq!(upload_script_for(ServerMode::LinuxNative, "x"), upload_script("x"));
        assert_eq!(ps_quote("a'b"), "'a''b'");
    }

    #[test]
    fn windows_env_init_8_char_vnc_password() {
        let s = env_init_script(ServerMode::Windows);
        for needle in ["windows-native", "RandomNumberGenerator", "SMT5_WEB_PASSWORD", "SMT5_API_TOKEN", "icacls", "/inheritance:r", "-lt 8", "-lt 248", "% 62", "'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'", ".env.example"] {
            assert!(s.contains(needle), "missing {needle}\n{s}");
        }
        assert!(s.contains("[Console]::Error.WriteLine($_.Exception.Message)\nexit 1") && !s.contains("Write-Error"));
        assert!(!s.contains("Write-Output $pw") && !s.contains("Write-Host $"), "no secret echo");
        let r = read_env_script_for(ServerMode::Windows);
        assert!(r.contains("LINGER=yes") && r.contains("VERSION="), "{r}");
        // installed = smt5.ps1 install finished (state\installed), not just uploaded
        assert!(r.contains("'state\\installed'") && r.contains("'INSTALLED=yes'") && r.contains("'INSTALLED=no'"), "{r}");
        // the marker must hold the digest in VERSION (a failed install during an update keeps the old one)
        assert!(r.contains("(([string]$m).Trim() -eq ([string]$v).Trim())"), "{r}");
        assert!(r.find("$v = Get-Content").unwrap() < r.find("'INSTALLED=yes'").unwrap(), "{r}");
        assert_eq!(read_env_script_for(ServerMode::Docker), read_env_script());
        assert!(!read_env_script().contains("INSTALLED"), "Linux unchanged");
    }

    #[test]
    fn win_reboot_needs_no_runtime() {
        // "Restart now" on the precheck runs before anything was uploaded
        let s = win_reboot_script();
        assert!(!s.contains("smt5.ps1") && !s.contains("siliconmetatrader5\\runtime"), "{s}");
        for needle in ["WindowsBuiltInRole]::Administrator", "not_admin", "shutdown.exe", "'/r', '/f', '/t', '5', '/d', 'p:4:1', '/c', 'SiliconMetaTrader5'", "1190", "'rebooting'", "exit 1"] {
            assert!(s.contains(needle), "missing {needle}\n{s}");
        }
    }

    #[test]
    fn stats_win_line() {
        let st = parse_stats("SMT5STATS win 37.5 1073741824 8589934592").unwrap();
        assert!((st.cpu_percent - 37.5).abs() < 1e-9 && (st.mem_percent - 12.5).abs() < 1e-9);
        assert_eq!(st.mem_used_bytes, 1073741824);
        assert!(parse_stats("SMT5STATS win 0 0 8589934592").is_none());
        assert!(parse_stats("SMT5STATS win 1 2").is_none());
        assert!(parse_stats("\u{feff}SMT5STATS win 37.5 1073741824 8589934592\r\n").is_some(), "BOM from PowerShell");
    }

    #[test]
    fn runtime_tarball_includes_windows() {
        let repo_rt = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../runtime");
        let tmp = std::env::temp_dir().join(format!("smt5-tarwin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let tgz = tmp.join("rt.tgz");
        runtime_tarball(&RuntimeDir::new(&repo_rt), &tgz, &SystemRunner).unwrap();
        let list = std::process::Command::new("tar").arg("-tzf").arg(&tgz).output().unwrap();
        let names = String::from_utf8_lossy(&list.stdout);
        for f in [
            "runtime/windows/smt5.ps1",
            "runtime/windows/versions.json",
            "runtime/windows/preflight.ps1",
            "runtime/windows/lib/common.ps1",
            "runtime/windows/lib/download.ps1",
            "runtime/windows/lib/autologon.ps1",
            "runtime/windows/lib/tasks.ps1",
            "runtime/windows/lib/settings.ps1",
            "runtime/windows/lib/install.ps1",
            "runtime/windows/supervisor.py",
            "runtime/windows/display.py",
            "runtime/config/requirements-windows.txt",
            "runtime/config/mt5cfg.ini",
        ] {
            assert!(names.lines().any(|l| l == f), "{f} missing:\n{names}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
