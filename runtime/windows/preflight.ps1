# Read-only server check for SiliconMetaTrader5 (Windows). Prints ONE compact JSON line.
# Runs as a scriptblock fed on stdin; Windows PowerShell 5.1 compatible. Changes nothing.
$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'

function Probe($block, $default) {
    try {
        $v = & $block
        if ($null -eq $v) { return $default }
        return $v
    } catch { return $default }
}

$build = [int](Probe { [Environment]::OSVersion.Version.Build } 0)
$arch = [string](Probe { $env:PROCESSOR_ARCHITECTURE } '')
$os_caption = [string](Probe { (Get-CimInstance Win32_OperatingSystem).Caption } '')
$admin = [bool](Probe {
    ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
} $false)

$disk_free_mb = [int64](Probe {
    $sd = $env:SystemDrive
    if (-not $sd) { $sd = 'C:' }
    [math]::Floor((Get-CimInstance Win32_LogicalDisk -Filter ("DeviceID='" + $sd + "'")).FreeSpace / 1MB)
} 0)
$mem_mb = [int64](Probe {
    [math]::Floor((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1MB)
} 0)
$ps_major = [int](Probe { $PSVersionTable.PSVersion.Major } 0)
$tar = [bool](Probe { $null -ne (Get-Command tar.exe -ErrorAction SilentlyContinue) } $false)

# network: HEAD to the three download hosts over TLS 1.2, 10 s each; true only if all succeed
$network = [bool](Probe {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $ok = $true
    foreach ($h in @('www.python.org', 'www.tightvnc.com', 'download.mql5.com')) {
        try {
            $req = [Net.WebRequest]::Create('https://' + $h + '/')
            $req.Method = 'HEAD'
            $req.Timeout = 10000
            $resp = $req.GetResponse()
            $resp.Close()
        } catch [Net.WebException] {
            # an HTTP status answer (403, 405...) still proves the host is reachable
            if ($null -eq $_.Exception.Response) { $ok = $false }
        } catch { $ok = $false }
    }
    $ok
} $false)

$pending_reboot = [bool](Probe {
    (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending') -or
    (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired')
} $false)

$logon_banner = [bool](Probe {
    $k = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System' -ErrorAction SilentlyContinue
    (-not [string]::IsNullOrEmpty($k.legalnoticecaption)) -or (-not [string]::IsNullOrEmpty($k.legalnoticetext))
} $false)

$foreign_mt5 = [bool](Probe {
    $d = 'C:\Program Files\MetaTrader 5'
    (Test-Path (Join-Path $d 'terminal64.exe')) -and (-not (Test-Path (Join-Path $d '.siliconmt5')))
} $false)

$version = $null
$installed = $false
try {
    $vf = Join-Path $env:LOCALAPPDATA 'siliconmetatrader5\VERSION'
    if (Test-Path $vf) {
        $installed = $true
        $version = ([IO.File]::ReadAllText($vf)).Trim()
    }
} catch { }

$result = [ordered]@{
    build          = $build
    arch           = $arch
    os_caption     = $os_caption
    admin          = $admin
    disk_free_mb   = $disk_free_mb
    mem_mb         = $mem_mb
    ps_major       = $ps_major
    tar            = $tar
    network        = $network
    pending_reboot = $pending_reboot
    logon_banner   = $logon_banner
    foreign_mt5    = $foreign_mt5
    installed      = $installed
    version        = $version
}
[Console]::Out.WriteLine(($result | ConvertTo-Json -Compress))
