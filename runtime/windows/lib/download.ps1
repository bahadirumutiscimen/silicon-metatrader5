# Pinned downloads (runtime\windows\versions.json), dot-sourced by smt5.ps1.
# Every file is verified before use: sha256 when pinned, otherwise a valid
# Authenticode signature from the expected signer; neither → refused.

function Get-Pinned([string]$Name) {
    $vf = Join-Path $WinDir 'versions.json'
    $all = ConvertFrom-Json -InputObject ([IO.File]::ReadAllText($vf))
    $e = $all.$Name
    if ($null -eq $e) { throw ('versions.json: "' + $Name + '" yok') }
    if (-not ([string]$e.url).StartsWith('https://')) { throw ('versions.json: "' + $Name + '" adresi https değil') }
    return $e
}

# Checks $Path against the pin; a file that fails is deleted.
function Assert-Verified([string]$Path, [string]$Sha256, [string]$Signer) {
    $leaf = Split-Path -Leaf $Path
    if (-not [string]::IsNullOrWhiteSpace($Sha256)) {
        $h = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
        if ($h -ine $Sha256.Trim()) {
            Remove-Item -LiteralPath $Path -Force
            throw ('sha256 uyuşmuyor: ' + $leaf)
        }
        Write-Log ('sha256 doğru: ' + $leaf)
        return
    }
    if (-not [string]::IsNullOrWhiteSpace($Signer)) {
        $sig = Get-AuthenticodeSignature -LiteralPath $Path
        $subject = ''
        if ($null -ne $sig.SignerCertificate) { $subject = [string]$sig.SignerCertificate.Subject }
        if (([string]$sig.Status -ne 'Valid') -or ($subject.IndexOf($Signer.Trim(), [StringComparison]::OrdinalIgnoreCase) -lt 0)) {
            Remove-Item -LiteralPath $Path -Force
            throw ('imza doğrulanamadı: ' + $leaf + ' (durum: ' + $sig.Status + '; imzalayan: "' + $subject + '"; beklenen: "' + $Signer + '")')
        }
        Write-Log ('imza doğru: ' + $leaf + ' (' + $subject + ')')
        return
    }
    Remove-Item -LiteralPath $Path -Force
    throw ('doğrulama bilgisi yok (versions.json sha256/signer boş): ' + $leaf)
}

# Downloads the pinned file $Name to downloads\$FileName (3 tries, TLS 1.2),
# verifies it and returns its path.
function Save-Pinned([string]$Name, [string]$FileName) {
    $e = Get-Pinned $Name
    $sha = [string]$e.sha256
    $signer = [string]$e.signer
    if ([string]::IsNullOrWhiteSpace($sha) -and [string]::IsNullOrWhiteSpace($signer)) {
        throw ('versions.json: "' + $Name + '" için sha256 ya da signer yok; doğrulanamayan dosya kurulmaz')
    }
    New-Item -ItemType Directory -Force -Path $DownloadDir | Out-Null
    $dest = Join-Path $DownloadDir $FileName
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'
    $err = ''
    for ($i = 1; $i -le 3; $i++) {
        try {
            if (Test-Path -LiteralPath $dest) { Remove-Item -LiteralPath $dest -Force }
            Write-Log ('indiriliyor (' + $i + '/3): ' + $e.url)
            Invoke-WebRequest -Uri ([string]$e.url) -OutFile $dest -UseBasicParsing -TimeoutSec 900
            $err = ''
            break
        } catch {
            $err = $_.Exception.Message
            Write-Log ('indirme denemesi ' + $i + ' başarısız: ' + $err)
            if ($i -lt 3) { Start-Sleep -Seconds (5 * $i) }
        }
    }
    if ($err -ne '') { throw ('indirilemedi: ' + $e.url + ' (' + $err + ')') }
    Assert-Verified -Path $dest -Sha256 $sha -Signer $signer
    return $dest
}
