# Screen saver and screen lock off for this user (dot-sourced by smt5.ps1):
# the automatically logged-on desktop must stay usable in noVNC. The values
# found at the first install are kept in state\prev-settings.json and put
# back by uninstall.

# IfPresent: a policy value is changed only when it is set (absent = no policy).
$SessionSettings = @(
    @{ Path = 'HKCU:\Control Panel\Desktop'; Name = 'ScreenSaveActive'; Value = '0'; Kind = 'String'; IfPresent = $false },
    @{ Path = 'HKCU:\Control Panel\Desktop'; Name = 'ScreenSaverIsSecure'; Value = '0'; Kind = 'String'; IfPresent = $false },
    @{ Path = 'HKCU:\Software\Policies\Microsoft\Windows\Control Panel\Desktop'; Name = 'ScreenSaveActive'; Value = '0'; Kind = 'String'; IfPresent = $true },
    @{ Path = 'HKCU:\Software\Policies\Microsoft\Windows\Control Panel\Desktop'; Name = 'ScreenSaverIsSecure'; Value = '0'; Kind = 'String'; IfPresent = $true },
    @{ Path = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System'; Name = 'InactivityTimeoutSecs'; Value = 0; Kind = 'DWord'; IfPresent = $true }
)

function Test-RegValue([string]$Path, [string]$Name) {
    if (-not (Test-Path -LiteralPath $Path)) { return $false }
    return ((Get-Item -LiteralPath $Path).GetValueNames() -contains $Name)
}

# First install only: a re-install must not record our own values as "previous".
function Save-PrevSettings {
    if (Test-Path -LiteralPath $PrevSettings) { Write-Log 'önceki ayarlar zaten kayıtlı (değiştirilmedi)'; return }
    $list = @()
    foreach ($s in $SessionSettings) {
        $exists = Test-RegValue $s.Path $s.Name
        $kind = $s.Kind
        $value = $null
        if ($exists) {
            $key = Get-Item -LiteralPath $s.Path
            $value = $key.GetValue($s.Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $kind = [string]$key.GetValueKind($s.Name)
        }
        $list += [pscustomobject]@{ path = $s.Path; name = $s.Name; exists = $exists; kind = $kind; value = $value }
    }
    New-Item -ItemType Directory -Force -Path $StateDir | Out-Null
    Write-TextFile $PrevSettings (ConvertTo-Json -InputObject @($list) -Depth 3)
    Write-Log 'önceki ekran koruyucu/kilit ayarları kaydedildi (prev-settings.json)'
}

function Set-SessionSettings {
    Save-PrevSettings
    foreach ($s in $SessionSettings) {
        if ($s.IfPresent -and -not (Test-RegValue $s.Path $s.Name)) { continue }
        if (-not (Test-Path -LiteralPath $s.Path)) { New-Item -Path $s.Path -Force | Out-Null }
        New-ItemProperty -LiteralPath $s.Path -Name $s.Name -Value $s.Value -PropertyType $s.Kind -Force | Out-Null
    }
    Write-Info 'Ekran koruyucu ve ekran kilidi bu kullanıcı için kapatıldı (kaldırınca eski ayarlar geri gelir).'
}

# Puts back what Save-PrevSettings found; → $true when everything was restored.
function Restore-PrevSettings {
    if (-not (Test-Path -LiteralPath $PrevSettings)) { Write-Log 'prev-settings.json yok; ayarlar geri yüklenmedi'; return $true }
    $all = $true
    $data = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($PrevSettings))
    foreach ($e in $data) {
        try {
            if ($e.exists) {
                if (-not (Test-Path -LiteralPath $e.path)) { New-Item -Path $e.path -Force | Out-Null }
                $v = $e.value
                if ($e.kind -eq 'DWord') { $v = [int]$v } elseif ($e.kind -eq 'QWord') { $v = [long]$v } else { $v = [string]$v }
                New-ItemProperty -LiteralPath $e.path -Name $e.name -Value $v -PropertyType $e.kind -Force | Out-Null
            } elseif (Test-RegValue $e.path $e.name) {
                Remove-ItemProperty -LiteralPath $e.path -Name $e.name
            }
        } catch {
            $all = $false
            Write-Log ('ayar geri yüklenemedi: ' + $e.path + ' ' + $e.name + ': ' + $_.Exception.Message)
        }
    }
    Write-Log 'ekran koruyucu/kilit ayarları geri yüklendi'
    return $all
}
