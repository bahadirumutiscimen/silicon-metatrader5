# Scheduled tasks and start/stop of the supervisor (dot-sourced by smt5.ps1).
#   SiliconMT5-Supervisor: at this user's logon, in the user's desktop session
#     (MT5's window must be on that desktop), pythonw.exe supervisor.py.
#   SiliconMT5-Console: as SYSTEM, when an RDP session is disconnected
#     (LocalSessionManager event 24), moves this user's session back to the
#     console with tscon so the desktop (MT5, VNC) keeps running unlocked.
#   SiliconMT5-Display: as SYSTEM, at startup, display.py → websockify (noVNC)
#     in front of TightVNC on 127.0.0.1. Independent of a user session: with
#     nobody logged on the screen shows the Windows logon screen. stop leaves
#     it running; uninstall ends and deletes it.
# supervisor.py contract: it exits when state\stop.request exists (and deletes
# it); it writes state\pids.json {"mt5","bridge","status","supervisor"}.
# display.py writes state\display.json {"launcher","websockify"}.

# Runs in the Console task (SYSTEM); __USER__ is replaced at registration.
# Only a session that is still disconnected after 5 s is moved: event 24 also
# fires when an RDP client takes the session over from the console, and that
# RDP login must not be undone. quser prints no SESSIONNAME for a disconnected
# session, so its second column is the id (state words are localized).
$ConsoleScriptTemplate = @'
$u = '__USER__'
Start-Sleep -Seconds 5
foreach ($l in @(& quser.exe 2>$null)) {
    $t = @(([string]$l).Trim().TrimStart('>').Trim() -split '\s+')
    if ($t.Count -lt 3 -or $t[0] -ine $u) { continue }
    if ($t[1] -match '^[0-9]+$') { & tscon.exe $t[1] /dest:console }
}
'@

function Register-Smt5Tasks {
    $me = Get-CurrentUser
    $pyw = Join-Path $PyDir 'pythonw.exe'
    $sup = Join-Path $WinDir 'supervisor.py'
    $action = New-ScheduledTaskAction -Execute $pyw -Argument ('"' + $sup + '"') -WorkingDirectory $Root
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $me.Full
    # Highest: MT5 (/portable) writes into its Program Files folder
    $principal = New-ScheduledTaskPrincipal -UserId $me.Full -LogonType Interactive -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $SupervisorTask -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'SiliconMetaTrader5: MT5, bridge, status API, noVNC' -Force | Out-Null
    Write-Log ('görev kaydedildi: ' + $SupervisorTask + ' (' + $me.Full + ')')

    # user name written into the action, single-quoted (a ' inside is doubled)
    $script = $ConsoleScriptTemplate.Replace('__USER__', $me.User.Replace("'", "''"))
    $b64 = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    $a2 = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -EncodedCommand ' + $b64)
    $cls = Get-CimClass -ClassName MSFT_TaskEventTrigger -Namespace 'Root/Microsoft/Windows/TaskScheduler'
    $t2 = New-CimInstance -CimClass $cls -ClientOnly
    $t2.Enabled = $true
    $t2.Subscription = '<QueryList><Query Id="0" Path="Microsoft-Windows-TerminalServices-LocalSessionManager/Operational"><Select Path="Microsoft-Windows-TerminalServices-LocalSessionManager/Operational">*[System[(EventID=24)]]</Select></Query></QueryList>'
    # S-1-5-18 = LocalSystem (the account name is localized on some Windows languages)
    $p2 = New-ScheduledTaskPrincipal -UserId 'S-1-5-18' -LogonType ServiceAccount -RunLevel Highest
    $s2 = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 5) -MultipleInstances Parallel -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $ConsoleTask -Action $a2 -Trigger $t2 -Principal $p2 -Settings $s2 -Description 'SiliconMetaTrader5: RDP disconnect -> session back to the console' -Force | Out-Null
    Write-Log ('görev kaydedildi: ' + $ConsoleTask)
    Register-DisplayTask
}

# display.py's arguments: the log limit (SMT5_LOG_MAX_MB, default 10; the
# task runs as SYSTEM, which cannot read the user-only .env), websockify's web
# root, listen and VNC addresses (SMT5_DISPLAY_PORT and +1000, both 127.0.0.1).
# Sync-DisplayTask compares them with the registered task.
function Get-DisplayArguments {
    $envTable = Read-DotEnv
    $port = Get-EnvPort $envTable 'SMT5_DISPLAY_PORT' 6081
    # as on Linux: digits only (else 10), at least 1
    $mb = 10
    $v = [string]$envTable['SMT5_LOG_MAX_MB']
    if ($v -match '^[0-9]{1,6}$') { $mb = [Math]::Max(1, [int]$v) }
    return ('"' + (Join-Path $WinDir 'display.py') + '" --log-max-mb ' + $mb + ' --web "' + $NovncDir + '"' + ' 127.0.0.1:' + $port + ' 127.0.0.1:' + ($port + 1000))
}

function Register-DisplayTask {
    $pyw = Join-Path $PyDir 'pythonw.exe'
    $action = New-ScheduledTaskAction -Execute $pyw -Argument (Get-DisplayArguments) -WorkingDirectory $Root
    $trigger = New-ScheduledTaskTrigger -AtStartup
    # S-1-5-18 = LocalSystem: runs before and without any logon
    $principal = New-ScheduledTaskPrincipal -UserId 'S-1-5-18' -LogonType ServiceAccount -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $DisplayTask -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Description 'SiliconMetaTrader5: noVNC (websockify) for the in-app screen, 127.0.0.1 only' -Force | Out-Null
    Write-Log ('görev kaydedildi: ' + $DisplayTask)
}

function Test-DisplayRunning {
    if ($null -ne (Get-OurProcess (Read-DisplayInfo).launcher)) { return $true }
    $t = Get-ScheduledTask -TaskName $DisplayTask -ErrorAction SilentlyContinue
    return ($null -ne $t -and [string]$t.State -eq 'Running')
}

function Start-DisplayTask {
    $r = Invoke-Native 'schtasks.exe' @('/Run', '/TN', $DisplayTask)
    if ($r.Code -ne 0) { throw ('ekran görevi başlatılamadı (schtasks /Run ' + $r.Code + '): ' + $r.Output) }
    Write-Log ('ekran görevi başlatıldı: ' + $DisplayTask)
}

# The launcher's process tree first (taskkill /T): ending only the task could
# leave websockify behind, holding the port.
function Stop-DisplayTask {
    $info = Read-DisplayInfo
    foreach ($name in @('launcher', 'websockify')) {
        $p = Get-OurProcess $info[$name]
        if ($null -eq $p) { continue }
        Write-Log ('ekran kapatılıyor: ' + $name + ' (pid ' + $p.Id + ')')
        $r = Invoke-Native 'taskkill.exe' @('/T', '/F', '/PID', [string]$p.Id)
        if ($r.Code -ne 0) { Write-Log ('taskkill ' + $name + ' (' + $r.Code + '): ' + $r.Output) }
    }
    if ($null -ne (Get-ScheduledTask -TaskName $DisplayTask -ErrorAction SilentlyContinue)) {
        $r = Invoke-Native 'schtasks.exe' @('/End', '/TN', $DisplayTask)
        Write-Log ('schtasks /End ' + $DisplayTask + ': ' + $r.Code)
    }
    Remove-PathSafe $DisplayFile
}

# The Display task with the ports and log limit in .env, running: registered
# again when its arguments differ, started when it is not running.
function Sync-DisplayTask {
    $t = Get-ScheduledTask -TaskName $DisplayTask -ErrorAction SilentlyContinue
    if ($null -eq $t) { return }
    $want = Get-DisplayArguments
    if ([string]$t.Actions[0].Arguments -ne $want) {
        Write-Info 'ekran ayarları (port, log sınırı) değişti; ekran görevi yeniden kaydediliyor'
        Stop-DisplayTask
        Register-DisplayTask
    }
    if (-not (Test-DisplayRunning)) { Start-DisplayTask }
}

function Unregister-Smt5Tasks {
    Stop-DisplayTask
    foreach ($n in @($SupervisorTask, $ConsoleTask, $DisplayTask)) {
        if ($null -ne (Get-ScheduledTask -TaskName $n -ErrorAction SilentlyContinue)) {
            Unregister-ScheduledTask -TaskName $n -Confirm:$false
            Write-Log ('görev silindi: ' + $n)
        }
    }
}

# stop.request → wait up to 20 s for the supervisor → taskkill the trees still
# alive (pids.json) → end the task → our leftover processes. Throws when any
# of our processes is still running. The Display task (screen) keeps running.
function Stop-Smt5 {
    New-Item -ItemType Directory -Force -Path $StateDir | Out-Null
    $pids = Read-Pids
    Write-TextFile $StopRequest ((Get-Date).ToString('o', $Inv))
    Write-Log 'stop.request yazıldı'
    $sup = 0
    if ($pids.ContainsKey('supervisor')) { $sup = $pids['supervisor'] }
    if ($null -ne (Get-OurProcess $sup)) {
        Write-Info 'durduruluyor (gözetmen en çok 20 sn bekleniyor)'
        $sw = [Diagnostics.Stopwatch]::StartNew()
        while (($sw.Elapsed.TotalSeconds -lt 20) -and ($null -ne (Get-OurProcess $sup))) { Start-Sleep -Milliseconds 500 }
    }
    foreach ($name in @($pids.Keys)) {
        $p = Get-OurProcess $pids[$name]
        if ($null -eq $p) { continue }
        Write-Log ('kapatılıyor: ' + $name + ' (pid ' + $p.Id + ')')
        $r = Invoke-Native 'taskkill.exe' @('/T', '/F', '/PID', [string]$p.Id)
        if ($r.Code -ne 0) { Write-Log ('taskkill ' + $name + ' (' + $r.Code + '): ' + $r.Output) }
    }
    $r = Invoke-Native 'schtasks.exe' @('/End', '/TN', $SupervisorTask)
    Write-Log ('schtasks /End: ' + $r.Code)
    foreach ($p in @(Get-StrayProcesses -ExceptDisplay)) {
        Write-Log ('kalan süreç kapatılıyor: ' + $p.ProcessName + ' (pid ' + $p.Id + ')')
        $r = Invoke-Native 'taskkill.exe' @('/T', '/F', '/PID', [string]$p.Id)
    }
    $left = @()
    for ($i = 0; $i -lt 10; $i++) {
        $left = @(Get-StrayProcesses -ExceptDisplay)
        if ($left.Count -eq 0) { break }
        Start-Sleep -Milliseconds 500
    }
    if ($left.Count -gt 0) {
        $names = ($left | ForEach-Object { $_.ProcessName + ' (pid ' + $_.Id + ')' }) -join ', '
        throw ('durdurulamadı: ' + $names)
    }
    Remove-PathSafe $StopRequest
    Remove-PathSafe $PidsFile
    Write-Info 'durdu'
}

function Start-Smt5 {
    if ($null -eq (Get-ScheduledTask -TaskName $SupervisorTask -ErrorAction SilentlyContinue)) {
        throw ('görev yok: ' + $SupervisorTask + ' (önce kurulum: smt5.ps1 install)')
    }
    # the screen first: it works without a session (logon screen)
    try { Sync-DisplayTask } catch { Write-Info ('uyarı: ekran görevi: ' + $_.Exception.Message) }
    if (-not (Test-UserSession)) {
        Write-Info 'Windows oturumu açık değil; yeniden başlatma ya da giriş bekleniyor'
        return
    }
    if (Test-SupervisorRunning) { Write-Info 'zaten çalışıyor'; return }
    Remove-PathSafe $StopRequest
    $r = Invoke-Native 'schtasks.exe' @('/Run', '/TN', $SupervisorTask)
    if ($r.Code -ne 0) { throw ('schtasks /Run başarısız (' + $r.Code + '): ' + $r.Output) }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt 15) {
        if (Test-SupervisorRunning) { Write-Info 'başlatıldı: MT5 penceresi birazdan açılır'; return }
        Start-Sleep -Milliseconds 500
    }
    throw ('gözetmen başlamadı (15 sn); ayrıntı: "' + $LogDir + '"')
}
