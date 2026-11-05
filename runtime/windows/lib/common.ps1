# Shared paths and helpers for smt5.ps1 (dot-sourced). Windows PowerShell 5.1.
# Paths are built with Join-Path and passed as variables or in double quotes:
# the user profile may contain spaces or non-ASCII letters.

$Root = Join-Path $env:LOCALAPPDATA 'siliconmetatrader5'
$Rt = Join-Path $Root 'runtime'
$WinDir = Join-Path $Rt 'windows'
$StateDir = Join-Path $Root 'state'
$LogDir = Join-Path $Root 'logs'
$PyDir = Join-Path $Root 'python'
$PyExe = Join-Path $PyDir 'python.exe'
$NovncDir = Join-Path $Root 'novnc'
$DownloadDir = Join-Path $Root 'downloads'
$EnvFile = Join-Path $Rt '.env'
$PidsFile = Join-Path $StateDir 'pids.json'
$StopRequest = Join-Path $StateDir 'stop.request'
$DisplayFile = Join-Path $StateDir 'display.json'
$InstalledFile = Join-Path $StateDir 'installed'
$VersionFile = Join-Path $Root 'VERSION'
$PrevSettings = Join-Path $StateDir 'prev-settings.json'
$ProgFiles = $env:ProgramW6432
if (-not $ProgFiles) { $ProgFiles = $env:ProgramFiles }
$Mt5Dir = Join-Path $ProgFiles 'MetaTrader 5'
$Mt5Exe = Join-Path $Mt5Dir 'terminal64.exe'
$Mt5Marker = Join-Path $Mt5Dir '.siliconmt5'
$Mt5Installing = Join-Path $StateDir 'mt5-installing'
$AutologonOurs = Join-Path $StateDir 'autologon-ours'
$SupervisorTask = 'SiliconMT5-Supervisor'
$ConsoleTask = 'SiliconMT5-Console'
$DisplayTask = 'SiliconMT5-Display'
$Inv = [Globalization.CultureInfo]::InvariantCulture
$Utf8 = New-Object Text.UTF8Encoding($false)
$script:NoLog = $false
# set by install: its lines also go to logs\installer.log (the Logs page)
$script:InstallerLog = $null

# One timestamped line in a log file under logs\ (never secrets). Rotates at 5 MB.
function Add-LogLine([string]$File, [string]$Message) {
    try {
        if (-not (Test-Path -LiteralPath $LogDir)) { New-Item -ItemType Directory -Force -Path $LogDir | Out-Null }
        if ((Test-Path -LiteralPath $File) -and ((Get-Item -LiteralPath $File).Length -gt 5MB)) {
            Move-Item -LiteralPath $File -Destination ($File + '.1') -Force
        }
        $ts = (Get-Date).ToString('yyyy-MM-dd HH:mm:ss', $Inv)
        [IO.File]::AppendAllText($File, ($ts + ' [' + $PID + '] ' + $Message + "`r`n"), $Utf8)
    } catch { }
}

# logs\smt5-ps.log: every command and step.
function Write-Log([string]$Message) {
    if ($script:NoLog) { return }
    Add-LogLine (Join-Path $LogDir 'smt5-ps.log') $Message
}

# A line for the user (job log / Terminal) and the log file.
function Write-Info([string]$Message) {
    [Console]::Out.WriteLine($Message)
    Write-Log $Message
    if ($script:InstallerLog -and -not $script:NoLog) { Add-LogLine $script:InstallerLog $Message }
}

# Runs a console program; stderr is captured instead of becoming a
# terminating error (Windows PowerShell 5.1 + 'Stop'). → @{Code; Output}
function Invoke-Native {
    param([string]$FilePath, [string[]]$ArgumentList = @())
    $ErrorActionPreference = 'Continue'
    $out = & $FilePath @ArgumentList 2>&1 | ForEach-Object { [string]$_ }
    $code = $LASTEXITCODE
    return [pscustomobject]@{ Code = $code; Output = ((@($out) -join "`n").Trim()) }
}

# Runs an installer with an exact argument line and waits; → exit code.
# $Shown is what the log gets instead of the arguments (they may hold a password).
function Invoke-Installer([string]$FilePath, [string]$ArgumentLine, [string]$Shown) {
    Write-Log ('çalıştırılıyor: ' + (Split-Path -Leaf $FilePath) + ' ' + $Shown)
    $p = Start-Process -FilePath $FilePath -ArgumentList $ArgumentLine -Wait -PassThru -WindowStyle Hidden
    Write-Log ('çıkış kodu: ' + $p.ExitCode)
    return [int]$p.ExitCode
}

function Remove-PathSafe([string]$Path) {
    if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Recurse -Force }
}

function Write-TextFile([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, $Utf8)
}

# runtime\.env → hashtable (values are never logged).
function Read-DotEnv {
    if (-not (Test-Path -LiteralPath $EnvFile)) { throw ('.env yok: "' + $EnvFile + '" (uygulama kurulumdan önce oluşturur)') }
    $h = @{}
    foreach ($l in [IO.File]::ReadAllLines($EnvFile)) {
        $t = $l.Trim()
        if ($t -eq '' -or $t.StartsWith('#')) { continue }
        $i = $t.IndexOf('=')
        if ($i -lt 1) { continue }
        $h[$t.Substring(0, $i).Trim()] = $t.Substring($i + 1).Trim().Trim('"')
    }
    return $h
}

function Get-EnvPort($EnvTable, [string]$Key, [int]$Default) {
    $n = 0
    if ($EnvTable.ContainsKey($Key) -and [int]::TryParse([string]$EnvTable[$Key], [ref]$n) -and $n -gt 0 -and $n -lt 65536) { return $n }
    return $Default
}

function Assert-Admin {
    $p = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'not_admin: bu komut yönetici (yükseltilmiş) bir SSH oturumu ister'
    }
}

# DOMAIN\user of this SSH session → @{Full; Domain; User}
function Get-CurrentUser {
    $name = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $i = $name.IndexOf('\')
    if ($i -ge 0) { return @{ Full = $name; Domain = $name.Substring(0, $i); User = $name.Substring($i + 1) } }
    return @{ Full = ($env:COMPUTERNAME + '\' + $name); Domain = $env:COMPUTERNAME; User = $name }
}

function Get-ProcPath($Process) {
    try { return [string]$Process.Path } catch { return '' }
}

# Is this executable ours (our python\ or our MT5 folder)? The MT5 folder
# counts only with our marker (or our install in progress): a foreign MT5 in
# the same place is never stopped or removed.
function Test-OurPath([string]$Path) {
    if ([string]::IsNullOrEmpty($Path)) { return $false }
    $dirs = @($PyDir)
    if ((Test-Path -LiteralPath $Mt5Marker) -or (Test-Path -LiteralPath $Mt5Installing)) { $dirs += $Mt5Dir }
    foreach ($d in $dirs) {
        if ($Path.StartsWith(($d.TrimEnd('\') + '\'), [StringComparison]::OrdinalIgnoreCase)) { return $true }
    }
    return $false
}

# The process with this id if it still runs one of our executables
# (pids.json may be stale after a reboot: never kill a reused pid).
function Get-OurProcess([int]$Id) {
    if ($Id -le 0) { return $null }
    $p = Get-Process -Id $Id -ErrorAction SilentlyContinue
    if ($null -eq $p) { return $null }
    if (Test-OurPath (Get-ProcPath $p)) { return $p }
    return $null
}

# Our python/pythonw/MT5 processes, wherever they came from. -ExceptDisplay
# leaves out the Display task's processes (stop keeps the screen running).
function Get-StrayProcesses([switch]$ExceptDisplay) {
    $names = @('python', 'pythonw', 'terminal64', 'metaeditor64')
    $skip = @{}
    if ($ExceptDisplay) { $skip = Get-DisplayTreeIds }
    return @(Get-Process -Name $names -ErrorAction SilentlyContinue | Where-Object { (Test-OurPath (Get-ProcPath $_)) -and -not $skip.ContainsKey([int]$_.Id) })
}

# state\display.json (written by display.py) → @{launcher; websockify}
function Read-DisplayInfo {
    $h = @{ launcher = 0; websockify = 0 }
    if (-not (Test-Path -LiteralPath $DisplayFile)) { return $h }
    try {
        $o = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($DisplayFile))
        foreach ($k in @('launcher', 'websockify')) {
            $n = 0
            if ([int]::TryParse([string]$o.$k, [ref]$n) -and $n -gt 0) { $h[$k] = $n }
        }
    } catch { Write-Log ('display.json okunamadı: ' + $_.Exception.Message) }
    return $h
}

# Process ids of the Display task: its launcher (display.py) and everything
# started under it (websockify and its per-connection children). A child
# older than the launcher has a reused parent id and is not one of them.
function Get-DisplayTreeIds {
    $ids = @{}
    $launcher = Get-OurProcess (Read-DisplayInfo).launcher
    if ($null -eq $launcher) { return $ids }
    $all = @(Get-CimInstance -ClassName Win32_Process -ErrorAction SilentlyContinue)
    $since = $null
    foreach ($p in $all) { if ([int]$p.ProcessId -eq $launcher.Id) { $since = $p.CreationDate } }
    $ids[[int]$launcher.Id] = $true
    do {
        $added = $false
        foreach ($p in $all) {
            $id = [int]$p.ProcessId
            if ($ids.ContainsKey($id) -or -not $ids.ContainsKey([int]$p.ParentProcessId)) { continue }
            if (($null -ne $since) -and ($p.CreationDate -lt $since)) { continue }
            $ids[$id] = $true
            $added = $true
        }
    } while ($added)
    return $ids
}

# state\pids.json (written by supervisor.py) → @{name = pid}
function Read-Pids {
    $h = @{}
    if (-not (Test-Path -LiteralPath $PidsFile)) { return $h }
    try {
        $o = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($PidsFile))
        foreach ($prop in $o.PSObject.Properties) {
            $n = 0
            if ([int]::TryParse([string]$prop.Value, [ref]$n) -and $n -gt 0) { $h[$prop.Name] = $n }
        }
    } catch { Write-Log ('pids.json okunamadı: ' + $_.Exception.Message) }
    return $h
}

# Does this user have a desktop session (console, RDP or disconnected)?
function Test-UserSession {
    $u = (Get-CurrentUser).User
    $procs = @(Get-CimInstance -ClassName Win32_Process -Filter "Name='explorer.exe' OR Name='sihost.exe'" -ErrorAction SilentlyContinue)
    foreach ($p in $procs) {
        if ($p.SessionId -eq 0) { continue }
        try { $o = Invoke-CimMethod -InputObject $p -MethodName GetOwner } catch { continue }
        if ($o.ReturnValue -eq 0 -and ([string]$o.User) -ieq $u) { return $true }
    }
    return $false
}

function Test-SupervisorRunning {
    $pids = Read-Pids
    if ($pids.ContainsKey('supervisor') -and ($null -ne (Get-OurProcess $pids['supervisor']))) { return $true }
    $t = Get-ScheduledTask -TaskName $SupervisorTask -ErrorAction SilentlyContinue
    return ($null -ne $t -and [string]$t.State -eq 'Running')
}
