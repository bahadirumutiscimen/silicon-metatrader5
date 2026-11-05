# Components: Python, pip packages, MT5, TightVNC, noVNC (dot-sourced by
# smt5.ps1). Each install step skips what is already at the pinned version;
# each uninstall step removes only what we installed.

$TvnOurs = Join-Path $StateDir 'tightvnc-ours'
$TvnPre = Join-Path $StateDir 'tightvnc-preexisting'

# ---------------------------------------------------------------- Python ---
function Install-Python {
    $pin = Get-Pinned 'python'
    $want = 'Python ' + $pin.version
    if (Test-Path -LiteralPath $PyExe) {
        if ((Invoke-Native $PyExe @('--version')).Output -eq $want) { Write-Info ($want + ' kurulu (atlandı)'); return }
    }
    Write-Info ($want + ' indiriliyor ve kuruluyor')
    $exe = Save-Pinned 'python' ('python-' + $pin.version + '-amd64.exe')
    $log = Join-Path $LogDir 'python-install.log'
    $line = '/quiet InstallAllUsers=0 TargetDir="' + $PyDir + '" PrependPath=0 Include_launcher=0 Shortcuts=0 Include_test=0 AssociateFiles=0 /log "' + $log + '"'
    $code = Invoke-Installer $exe $line $line
    Remove-Item -LiteralPath $exe -Force -ErrorAction SilentlyContinue
    if ($code -ne 0 -and $code -ne 3010) { throw ('Python kurulumu başarısız (çıkış ' + $code + '; "' + $log + '")') }
    $got = ''
    if (Test-Path -LiteralPath $PyExe) { $got = (Invoke-Native $PyExe @('--version')).Output }
    if ($got -ne $want) { throw ('Python kurulduktan sonra sürüm "' + $got + '", beklenen "' + $want + '" ("' + $log + '")') }
    Write-Info ($want + ' kuruldu')
}

function Install-PipPackages {
    $req = Join-Path $Rt 'config\requirements-windows.txt'
    Write-Info 'Python paketleri kuruluyor (MetaTrader5, siliconmetatrader5, websockify)'
    $r = Invoke-Native $PyExe @('-m', 'pip', 'install', '--disable-pip-version-check', '--no-warn-script-location', '--no-input', '-r', $req)
    Write-Log $r.Output
    if ($r.Code -ne 0) {
        $tail = (@($r.Output -split "`n") | Select-Object -Last 15) -join "`n"
        throw ('pip install başarısız (' + $r.Code + '):' + "`n" + $tail)
    }
}

# Uninstalls the per-user Python bundle when it is the one in our python\
# (keeps "Apps & features" clean), then deletes the folder.
function Uninstall-Python {
    if (-not (Test-Path -LiteralPath $PyDir)) { return }
    try {
        $ip = Get-ItemProperty -LiteralPath 'HKCU:\Software\Python\PythonCore\3.13\InstallPath' -ErrorAction SilentlyContinue
        $loc = ''
        if ($null -ne $ip) { $loc = [string]$ip.'(default)' }
        if ($loc -ne '' -and ($loc.TrimEnd('\') -ieq $PyDir.TrimEnd('\'))) {
            $base = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
            foreach ($k in @(Get-ChildItem -LiteralPath $base -ErrorAction SilentlyContinue)) {
                $dn = [string]$k.GetValue('DisplayName')
                $q = [string]$k.GetValue('QuietUninstallString')
                if (($dn -like 'Python 3.13.* (64-bit)') -and ($q -ne '')) {
                    Write-Info ($dn + ' kaldırılıyor')
                    $p = Start-Process -FilePath 'cmd.exe' -ArgumentList ('/c "' + $q + '"') -Wait -PassThru -WindowStyle Hidden
                    Write-Log ('Python kaldırıcı çıkış kodu: ' + $p.ExitCode)
                }
            }
        }
    } catch { Write-Log ('Python kaldırıcı çalışmadı: ' + $_.Exception.Message) }
    Remove-PathSafe $PyDir
}

# ------------------------------------------------------------------- MT5 ---
# mt5cfg.ini (MaxBars, Algo Trading, DLL imports from .env) is written by
# supervisor.py before each MT5 start, like native.sh sync_mt5cfg.

function Stop-Mt5Setup($Setup) {
    if ($null -ne $Setup -and -not $Setup.HasExited) {
        Invoke-Native 'taskkill.exe' @('/T', '/F', '/PID', [string]$Setup.Id) | Out-Null
    }
    foreach ($p in @(Get-Process -Name 'mt5setup' -ErrorAction SilentlyContinue)) {
        Invoke-Native 'taskkill.exe' @('/T', '/F', '/PID', [string]$p.Id) | Out-Null
    }
    # the installer starts the terminal when done; the supervisor starts it again with our config
    $term = @(Get-Process -Name 'terminal64' -ErrorAction SilentlyContinue | Where-Object { Test-OurPath (Get-ProcPath $_) })
    foreach ($p in $term) { try { $p.CloseMainWindow() | Out-Null } catch { } }
    if ($term.Count -gt 0) { Start-Sleep -Seconds 5 }
    foreach ($p in $term) { if (-not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue } }
}

function Install-Mt5 {
    if (Test-Path -LiteralPath $Mt5Marker) {
        Write-Info 'MetaTrader 5 kurulu (atlandı)'
        return
    }
    # a terminal without our marker belongs to someone else (preflight: foreign_mt5)
    if ((Test-Path -LiteralPath $Mt5Exe) -and -not (Test-Path -LiteralPath $Mt5Installing)) {
        throw ('foreign_mt5: "' + $Mt5Dir + '" bu uygulamanın kurmadığı bir MetaTrader 5 içeriyor; ona dokunulmaz. Kaldırın ya da başka bir sunucu kullanın.')
    }
    Write-TextFile $Mt5Installing ((Get-Date).ToString('o', $Inv))
    $setupExe = Save-Pinned 'mt5' 'mt5setup.exe'
    Write-Info 'MetaTrader 5 kurulum programı çalıştırılıyor (birkaç dakika)'
    $setup = Start-Process -FilePath $setupExe -ArgumentList '/auto' -PassThru
    $found = $false
    # up to 15 min for terminal64.exe, then up to 3 min for the installer to
    # finish copying; a line every 30 s keeps the job log (and ssh) alive
    for ($i = 0; $i -lt 90; $i++) {
        if (Test-Path -LiteralPath $Mt5Exe) { $found = $true; break }
        if ($i -gt 0 -and $i % 3 -eq 0) { Write-Info ('MetaTrader 5 kuruluyor… ' + ($i * 10) + ' sn') }
        Start-Sleep -Seconds 10
    }
    if ($found) {
        for ($i = 0; $i -lt 36; $i++) {
            if ($setup.HasExited) { break }
            if ($i -gt 0 -and $i % 6 -eq 0) { Write-Info ('MetaTrader 5 dosyaları kopyalanıyor… ' + ($i * 5) + ' sn') }
            Start-Sleep -Seconds 5
        }
    }
    Stop-Mt5Setup $setup
    Remove-Item -LiteralPath $setupExe -Force -ErrorAction SilentlyContinue
    if (-not $found) { throw ('MT5 kurulumu zaman aşımı: 15 dakikada "' + $Mt5Exe + '" görünmedi') }
    Write-TextFile $Mt5Marker ('siliconmetatrader5 ' + (Get-Date).ToString('o', $Inv) + "`n")
    Remove-PathSafe $Mt5Installing
    Write-Info ('MetaTrader 5 kuruldu: ' + $Mt5Dir)
}

# Only an MT5 with our marker: folder, its Apps & features entry, shortcuts.
function Uninstall-Mt5 {
    if (-not (Test-Path -LiteralPath $Mt5Marker)) {
        if (Test-Path -LiteralPath $Mt5Dir) { Write-Info 'MetaTrader 5 bu uygulamaya ait değil; dokunulmadı' }
        return
    }
    Remove-PathSafe $Mt5Dir
    $prefix = $Mt5Dir.TrimEnd('\') + '\'
    foreach ($base in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')) {
        foreach ($k in @(Get-ChildItem -LiteralPath $base -ErrorAction SilentlyContinue)) {
            $us = [string]$k.GetValue('UninstallString')
            if ($us.IndexOf($prefix, [StringComparison]::OrdinalIgnoreCase) -ge 0) { Remove-Item -LiteralPath $k.PSPath -Recurse -Force }
        }
    }
    foreach ($f in @('CommonDesktopDirectory', 'DesktopDirectory')) {
        Remove-PathSafe (Join-Path ([Environment]::GetFolderPath($f)) 'MetaTrader 5.lnk')
    }
    foreach ($f in @('CommonPrograms', 'Programs')) {
        Remove-PathSafe (Join-Path ([Environment]::GetFolderPath($f)) 'MetaTrader 5')
    }
    Write-Info 'MetaTrader 5 kaldırıldı'
}

# -------------------------------------------------------------- TightVNC ---
function Get-TightVncProduct {
    foreach ($base in @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')) {
        foreach ($k in @(Get-ChildItem -LiteralPath $base -ErrorAction SilentlyContinue)) {
            $dn = [string]$k.GetValue('DisplayName')
            if ($dn -like 'TightVNC*') {
                return [pscustomobject]@{ Code = $k.PSChildName; Name = $dn; Version = [string]$k.GetValue('DisplayVersion') }
            }
        }
    }
    return $null
}

# VNC password as TightVNC stores it (DES, the fixed VNC key, bits reversed).
function ConvertTo-VncPassword([string]$Plain) {
    $key = [byte[]](0xE8, 0x4A, 0xD6, 0x60, 0xC4, 0x72, 0x1A, 0xE0)
    $data = New-Object byte[] 8
    $src = [Text.Encoding]::ASCII.GetBytes($Plain)
    [Array]::Copy($src, $data, [Math]::Min(8, $src.Length))
    $des = New-Object Security.Cryptography.DESCryptoServiceProvider
    try {
        $des.Mode = [Security.Cryptography.CipherMode]::ECB
        $des.Padding = [Security.Cryptography.PaddingMode]::None
        $enc = $des.CreateEncryptor($key, (New-Object byte[] 8))
        $out = $enc.TransformFinalBlock($data, 0, 8)
        $enc.Dispose()
    } finally { $des.Dispose() }
    return ,$out
}

# TightVNC already at the pinned version (an update, or installed before us):
# the MSI would not run again, so port, password and loopback go to its settings.
function Set-TightVncConfig([int]$Port, [string]$Plain) {
    $k = 'HKLM:\SOFTWARE\TightVNC\Server'
    if (-not (Test-Path -LiteralPath $k)) { New-Item -Path $k -Force | Out-Null }
    $dw = [ordered]@{ RfbPort = $Port; UseVncAuthentication = 1; AllowLoopback = 1; LoopbackOnly = 1; AcceptHttpConnections = 0 }
    foreach ($n in $dw.Keys) { New-ItemProperty -LiteralPath $k -Name $n -Value $dw[$n] -PropertyType DWord -Force | Out-Null }
    New-ItemProperty -LiteralPath $k -Name 'Password' -Value (ConvertTo-VncPassword $Plain) -PropertyType Binary -Force | Out-Null
    $svc = Get-Service -Name 'tvnserver' -ErrorAction SilentlyContinue
    if ($null -ne $svc) { Restart-Service -Name 'tvnserver' -Force }
}

function Install-TightVnc($EnvTable) {
    $pin = Get-Pinned 'tightvnc'
    $vncPass = [string]$EnvTable['SMT5_WEB_PASSWORD']
    if ($vncPass -cnotmatch '^[A-Za-z0-9]{8}$') { throw 'SMT5_WEB_PASSWORD (.env) 8 harf/rakam olmalı' }
    $port = (Get-EnvPort $EnvTable 'SMT5_DISPLAY_PORT' 6081) + 1000
    $prod = Get-TightVncProduct
    if (($null -ne $prod) -and -not (Test-Path -LiteralPath $TvnOurs) -and -not (Test-Path -LiteralPath $TvnPre)) {
        Write-TextFile $TvnPre ($prod.Name + ' ' + $prod.Version + "`n")
        Write-Info ('TightVNC bu kurulumdan önce vardı (' + $prod.Name + '); kaldırırken dokunulmayacak')
    }
    if (($null -ne $prod) -and $prod.Version.StartsWith($pin.version)) {
        Set-TightVncConfig -Port $port -Plain $vncPass
        Write-Info ('TightVNC ' + $pin.version + ' kurulu (atlandı); port ' + $port + ', yalnız 127.0.0.1')
        return
    }
    Write-Info ('TightVNC ' + $pin.version + ' indiriliyor ve kuruluyor (servis, yalnız 127.0.0.1, port ' + $port + ')')
    $msi = Save-Pinned 'tightvnc' ('tightvnc-' + $pin.version + '-gpl-setup-64bit.msi')
    $line = '/i "' + $msi + '" /quiet /norestart ADDLOCAL=Server SERVER_REGISTER_AS_SERVICE=1 SERVER_ADD_FIREWALL_EXCEPTION=0 ' +
        'SET_USEVNCAUTHENTICATION=1 VALUE_OF_USEVNCAUTHENTICATION=1 SET_PASSWORD=1 VALUE_OF_PASSWORD=' + $vncPass + ' ' +
        'SET_RFBPORT=1 VALUE_OF_RFBPORT=' + $port + ' SET_ALLOWLOOPBACK=1 VALUE_OF_ALLOWLOOPBACK=1 ' +
        'SET_LOOPBACKONLY=1 VALUE_OF_LOOPBACKONLY=1 SET_ACCEPTHTTPCONNECTIONS=1 VALUE_OF_ACCEPTHTTPCONNECTIONS=0'
    $code = Invoke-Installer 'msiexec.exe' $line ('/i ' + (Split-Path -Leaf $msi) + ' (parametreler gizli)')
    Remove-Item -LiteralPath $msi -Force -ErrorAction SilentlyContinue
    if ($code -ne 0 -and $code -ne 3010) { throw ('TightVNC kurulumu başarısız (msiexec çıkış ' + $code + ')') }
    if (-not (Test-Path -LiteralPath $TvnPre)) { Write-TextFile $TvnOurs ($pin.version + "`n") }
    Write-Info ('TightVNC ' + $pin.version + ' kuruldu')
}

# Only a TightVNC our MSI installed (tightvnc-ours); without that marker it
# is someone else's (or our install never got that far): left alone.
function Uninstall-TightVnc {
    if (-not (Test-Path -LiteralPath $TvnOurs)) {
        if ($null -ne (Get-TightVncProduct)) { Write-Info 'TightVNC bu uygulamanın kurduğu değil; dokunulmadı' }
        return
    }
    $prod = Get-TightVncProduct
    if ($null -eq $prod) { return }
    if ($prod.Code -notmatch '^\{[0-9A-Fa-f-]{36}\}$') { throw ('TightVNC ürün kodu bulunamadı (' + $prod.Code + ')') }
    $code = Invoke-Installer 'msiexec.exe' ('/x ' + $prod.Code + ' /quiet /norestart') ('/x ' + $prod.Code)
    if (@(0, 3010, 1605) -notcontains $code) { throw ('TightVNC kaldırılamadı (msiexec çıkış ' + $code + ')') }
    Write-Info 'TightVNC kaldırıldı'
}

# ----------------------------------------------------------------- noVNC ---
function Install-Novnc {
    $pin = Get-Pinned 'novnc'
    $stamp = Join-Path $NovncDir '.smt5-version'
    if ((Test-Path -LiteralPath (Join-Path $NovncDir 'vnc.html')) -and (Test-Path -LiteralPath $stamp) -and
        (([IO.File]::ReadAllText($stamp)).Trim() -eq $pin.version)) {
        Write-Info ('noVNC ' + $pin.version + ' kurulu (atlandı)')
        return
    }
    Write-Info ('noVNC ' + $pin.version + ' indiriliyor')
    $zip = Save-Pinned 'novnc' ('noVNC-' + $pin.version + '.zip')
    $tmp = Join-Path $Root 'novnc.tmp'
    Remove-PathSafe $tmp
    Expand-Archive -LiteralPath $zip -DestinationPath $tmp -Force
    $top = Join-Path $tmp ('noVNC-' + $pin.version)
    if (-not (Test-Path -LiteralPath (Join-Path $top 'vnc.html'))) { throw ('noVNC arşivinde vnc.html yok: ' + (Split-Path -Leaf $zip)) }
    Remove-PathSafe $NovncDir
    Move-Item -LiteralPath $top -Destination $NovncDir
    Remove-PathSafe $tmp
    Remove-Item -LiteralPath $zip -Force -ErrorAction SilentlyContinue
    Write-TextFile $stamp ($pin.version + "`n")
    Write-Info ('noVNC ' + $pin.version + ' kuruldu')
}
