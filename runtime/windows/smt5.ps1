# SiliconMetaTrader5 on a Windows server: install and run MT5, the Python
# bridge, the status API and the in-app screen (TightVNC + noVNC).
#
#   smt5.ps1 install           folders, Python, pip packages, MT5, TightVNC, noVNC, tasks, settings (idempotent);
#                              state\installed (the VERSION digest) only when every step worked
#   smt5.ps1 start|stop|restart  the supervisor task (SiliconMT5-Supervisor); the screen task
#                              (SiliconMT5-Display) keeps running on stop, start re-syncs its port
#   smt5.ps1 reset             stop; delete MT5 data (login, history, logs) and state; programs stay
#   smt5.ps1 uninstall         stop (fails → nothing deleted); remove everything we installed
#   smt5.ps1 autologon [off]   ask the Windows password here (Terminal) → automatic logon (LSA secret)
#   smt5.ps1 autologon-status  on | off
#   smt5.ps1 session           session=yes|no task=yes|no
#   smt5.ps1 reboot            restart Windows in 5 s; prints "rebooting" at once
#   smt5.ps1 stats             SMT5STATS win <cpu%> <used bytes> <total bytes>
#
# Windows PowerShell 5.1. Exit code 0 = success; errors go to stderr.
# Everything listens on 127.0.0.1 only; no firewall rule is added.
param(
    [Parameter(Position = 0)][string]$Command = '',
    [Parameter(Position = 1)][string]$Arg = ''
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Install-Smt5 {
    Assert-Admin
    $envTable = Read-DotEnv
    foreach ($d in @($Root, $StateDir, $LogDir)) { New-Item -ItemType Directory -Force -Path $d | Out-Null }
    $script:InstallerLog = Join-Path $LogDir 'installer.log'
    Write-Info ('---- kurulum başladı (' + $env:COMPUTERNAME + ')')
    # websockify runs from our python\: off while Python and pip packages change
    Stop-DisplayTask
    Install-Python
    Install-PipPackages
    Install-Mt5
    Install-TightVnc $envTable
    Install-Novnc
    Register-Smt5Tasks
    Set-SessionSettings
    Start-DisplayTask
    Remove-PathSafe $DownloadDir
    $digest = 'unknown'
    if (Test-Path -LiteralPath $VersionFile) { $digest = ([IO.File]::ReadAllText($VersionFile)).Trim() }
    Write-TextFile $InstalledFile ($digest + "`n")
    Write-Info 'Kurulum tamam.'
    Write-Info 'Not: otomatik oturum açıkken sağlayıcı konsolunu (VPS paneli) açan biri Windows masaüstünü görür; bu yüzden ekran kilidi ve ekran koruyucu kapalıdır.'
}

function Reset-Smt5 {
    Stop-Smt5
    if (Test-Path -LiteralPath $Mt5Marker) {
        foreach ($rel in @('Config', 'bases', 'logs', (Join-Path 'MQL5' 'Files'), (Join-Path 'MQL5' 'Logs'), 'Tester')) {
            Remove-PathSafe (Join-Path $Mt5Dir $rel)
        }
    }
    # install-time records stay: uninstall needs them later
    $keep = @('prev-settings.json', 'tightvnc-preexisting', 'tightvnc-ours', 'autologon-ours', 'installed', 'display.json')
    if (Test-Path -LiteralPath $StateDir) {
        foreach ($item in @(Get-ChildItem -LiteralPath $StateDir -Force)) {
            if ($keep -notcontains $item.Name) { Remove-Item -LiteralPath $item.FullName -Recurse -Force }
        }
    }
    Write-Info 'MT5 verisi silindi (broker girişi, geçmiş, loglar); programlar yerinde. Başlat ile MT5 yeniden açılır.'
}

function Uninstall-Smt5 {
    Assert-Admin
    Stop-Smt5   # throws → exit 1 before anything is deleted
    $failed = @()
    $steps = [ordered]@{
        'görevler' = { Unregister-Smt5Tasks }
        'otomatik oturum' = {
            if (Test-Path -LiteralPath $AutologonOurs) { Disable-Autologon }
            else { Write-Info 'otomatik oturum bu uygulamanın açtığı değil; dokunulmadı' }
        }
        'ekran ayarları' = { if (-not (Restore-PrevSettings)) { throw 'bazı ayarlar geri yüklenemedi (smt5-ps.log)' } }
        'TightVNC' = { Uninstall-TightVnc }
        'MetaTrader 5' = { Uninstall-Mt5 }
        'Python' = { Uninstall-Python }
        'noVNC' = { Remove-PathSafe $NovncDir }
    }
    foreach ($name in $steps.Keys) {
        try { & $steps[$name] } catch {
            $failed += ($name + ': ' + $_.Exception.Message)
            Write-Log ('kaldırma adımı başarısız: ' + $name + ': ' + $_.Exception.Message)
        }
    }
    if ($failed.Count -gt 0) {
        throw ('kaldırma tamamlanmadı (klasör ve loglar duruyor: "' + $Root + '"): ' + ($failed -join '; '))
    }
    Set-Location -LiteralPath $env:USERPROFILE
    Write-Log ('klasör siliniyor: ' + $Root)
    $script:NoLog = $true
    Remove-PathSafe $Root
    [Console]::Out.WriteLine('removed')
}

function Get-SessionLine {
    $s = 'no'
    if (Test-UserSession) { $s = 'yes' }
    $t = 'no'
    if (Test-SupervisorRunning) { $t = 'yes' }
    return ('session=' + $s + ' task=' + $t)
}

# A restart scheduled by Windows itself (not a child of this SSH session, which
# ends with the connection): 5 s delay, forced, planned (no reason prompt).
function Invoke-Reboot {
    $r = Invoke-Native 'shutdown.exe' @('/r', '/f', '/t', '5', '/d', 'p:4:1', '/c', 'SiliconMetaTrader5')
    # 1190: a shutdown is already scheduled
    if ($r.Code -ne 0 -and $r.Code -ne 1190) { throw ('yeniden başlatılamadı (shutdown.exe ' + $r.Code + '): ' + $r.Output) }
    Write-Log 'yeniden başlatma 5 sn sonra'
    [Console]::Out.WriteLine('rebooting')
}

# CPU (per-core %, two samples 1 s apart) and working set of our processes:
# terminal64 in our MT5 folder, python/pythonw under our python\, tvnserver.
function Get-StatsLine {
    $sample = {
        $h = @{}
        foreach ($p in @(Get-Process -Name 'terminal64', 'tvnserver', 'python', 'pythonw' -ErrorAction SilentlyContinue)) {
            if (($p.ProcessName -ne 'tvnserver') -and -not (Test-OurPath (Get-ProcPath $p))) { continue }
            try { $h[$p.Id] = @{ Cpu = $p.TotalProcessorTime.TotalSeconds; Mem = [int64]$p.WorkingSet64 } } catch { }
        }
        return $h
    }
    $a = & $sample
    $sw = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Seconds 1
    $b = & $sample
    $elapsed = $sw.Elapsed.TotalSeconds
    $cpu = 0.0
    $used = [int64]0
    foreach ($id in $b.Keys) {
        $used += $b[$id].Mem
        if ($a.ContainsKey($id)) {
            $d = $b[$id].Cpu - $a[$id].Cpu
            if ($d -gt 0) { $cpu += $d }
        }
    }
    $pct = 0.0
    if ($elapsed -gt 0) { $pct = $cpu * 100.0 / $elapsed }
    $total = [int64](Get-CimInstance -ClassName Win32_ComputerSystem).TotalPhysicalMemory
    return [string]::Format($Inv, 'SMT5STATS win {0:0.0} {1} {2}', $pct, $used, $total)
}

try {
    $lib = Join-Path $PSScriptRoot 'lib'
    foreach ($f in @('common.ps1', 'download.ps1', 'autologon.ps1', 'tasks.ps1', 'settings.ps1', 'install.ps1')) {
        . (Join-Path $lib $f)
    }
    Write-Log ('smt5.ps1 ' + $Command + ' ' + $Arg)
    switch ($Command) {
        'install' { Install-Smt5 }
        'start' { Start-Smt5 }
        'stop' { Stop-Smt5 }
        'restart' { Stop-Smt5; Start-Smt5 }
        'reset' { Reset-Smt5 }
        'uninstall' { Uninstall-Smt5 }
        'autologon' {
            if ($Arg -eq 'off') { Assert-Admin; Disable-Autologon; Write-Info 'otomatik oturum kapatıldı' }
            elseif ($Arg -eq '' -or $Arg -eq 'on') { Enable-Autologon }
            else { throw ('autologon: bilinmeyen seçenek "' + $Arg + '" (off)') }
        }
        'autologon-status' { [Console]::Out.WriteLine((Get-AutologonStatus)) }
        'session' { [Console]::Out.WriteLine((Get-SessionLine)) }
        'reboot' { Assert-Admin; Invoke-Reboot }
        'stats' { [Console]::Out.WriteLine((Get-StatsLine)) }
        default {
            throw ('bilinmeyen komut: "' + $Command + '" (install|start|stop|restart|reset|uninstall|autologon|autologon-status|session|reboot|stats)')
        }
    }
    Write-Log ('tamam: ' + $Command)
} catch {
    $msg = $_.Exception.Message
    try { Write-Log ('HATA (' + $Command + '): ' + $msg) } catch { }
    if ($script:InstallerLog) { Add-LogLine $script:InstallerLog ('HATA: ' + $msg) }
    [Console]::Error.WriteLine($msg)
    exit 1
}
exit 0
