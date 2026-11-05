# Automatic logon (dot-sourced by smt5.ps1). The Windows password is typed by
# the user in their own Terminal (ssh -t), checked, and stored only as the LSA
# secret "DefaultPassword" (the Sysinternals Autologon method). It is never
# written to the registry in plain text, printed or logged.

$Winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'

function Initialize-Lsa {
    if ('Smt5Lsa' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class Smt5Lsa {
    [StructLayout(LayoutKind.Sequential)]
    private struct LSA_UNICODE_STRING { public ushort Length; public ushort MaximumLength; public IntPtr Buffer; }

    [StructLayout(LayoutKind.Sequential)]
    private struct LSA_OBJECT_ATTRIBUTES {
        public int Length; public IntPtr RootDirectory; public IntPtr ObjectName; public uint Attributes;
        public IntPtr SecurityDescriptor; public IntPtr SecurityQualityOfService;
    }

    [DllImport("advapi32.dll")]
    private static extern uint LsaOpenPolicy(IntPtr systemName, ref LSA_OBJECT_ATTRIBUTES attributes, uint access, out IntPtr policy);
    [DllImport("advapi32.dll")]
    private static extern uint LsaStorePrivateData(IntPtr policy, ref LSA_UNICODE_STRING keyName, IntPtr privateData);
    [DllImport("advapi32.dll")]
    private static extern uint LsaClose(IntPtr handle);
    [DllImport("advapi32.dll")]
    private static extern int LsaNtStatusToWinError(uint status);

    private const uint POLICY_ALL_ACCESS = 0x000F0FFF;
    private const uint STATUS_OBJECT_NAME_NOT_FOUND = 0xC0000034;

    // data: UTF-16 characters (e.g. a BSTR) and their count; IntPtr.Zero deletes the secret.
    // The characters are passed by pointer: no managed copy is made here.
    public static void Store(string key, IntPtr data, int chars) {
        LSA_OBJECT_ATTRIBUTES attrs = new LSA_OBJECT_ATTRIBUTES();
        attrs.Length = Marshal.SizeOf(typeof(LSA_OBJECT_ATTRIBUTES));
        IntPtr policy;
        uint st = LsaOpenPolicy(IntPtr.Zero, ref attrs, POLICY_ALL_ACCESS, out policy);
        if (st != 0) { throw new Win32Exception(LsaNtStatusToWinError(st)); }
        LSA_UNICODE_STRING k = new LSA_UNICODE_STRING();
        k.Buffer = Marshal.StringToHGlobalUni(key);
        k.Length = (ushort)(key.Length * 2);
        k.MaximumLength = (ushort)((key.Length + 1) * 2);
        IntPtr value = IntPtr.Zero;
        try {
            if (data != IntPtr.Zero) {
                LSA_UNICODE_STRING v = new LSA_UNICODE_STRING();
                v.Buffer = data;
                v.Length = (ushort)(chars * 2);
                v.MaximumLength = (ushort)(chars * 2);
                value = Marshal.AllocHGlobal(Marshal.SizeOf(typeof(LSA_UNICODE_STRING)));
                Marshal.StructureToPtr(v, value, false);
            }
            st = LsaStorePrivateData(policy, ref k, value);
            if (st != 0 && !(data == IntPtr.Zero && st == STATUS_OBJECT_NAME_NOT_FOUND)) {
                throw new Win32Exception(LsaNtStatusToWinError(st));
            }
        } finally {
            if (value != IntPtr.Zero) { Marshal.FreeHGlobal(value); }
            Marshal.FreeHGlobal(k.Buffer);
            LsaClose(policy);
        }
    }
}
'@
}

function Remove-WinlogonValue([string]$Name) {
    $k = Get-Item -LiteralPath $Winlogon
    if ($k.GetValueNames() -contains $Name) { Remove-ItemProperty -LiteralPath $Winlogon -Name $Name }
}

function Enable-Autologon {
    Assert-Admin
    $me = Get-CurrentUser
    $machine = ($me.Domain -ieq $env:COMPUTERNAME)
    Add-Type -AssemblyName System.DirectoryServices.AccountManagement
    Initialize-Lsa
    [Console]::Out.WriteLine('')
    [Console]::Out.WriteLine('SiliconMetaTrader5 - otomatik oturum / automatic logon: ' + $me.Full)
    [Console]::Out.WriteLine('Windows şifresi yalnız bu sunucuda, Windows''un şifreli deposunda (LSA) saklanır; uygulama görmez.')
    [Console]::Out.WriteLine('The Windows password stays on this server, in Windows'' protected store (LSA); the app never sees it.')
    [Console]::Out.WriteLine('Not / Note: sağlayıcı konsolunu (VPS paneli) açan biri masaüstünü görür / anyone with the provider console sees the desktop.')
    [Console]::Out.WriteLine('')
    $ok = $false
    for ($try = 1; ($try -le 3) -and (-not $ok); $try++) {
        $secure = Read-Host -AsSecureString -Prompt ('Windows şifresi / password (' + $me.User + ')')
        $bstr = [IntPtr]::Zero
        $ctx = $null
        try {
            if ($secure.Length -eq 0) {
                [Console]::Out.WriteLine('Şifre boş olamaz / Password must not be empty (' + $try + '/3)')
                continue
            }
            $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
            if ($machine) {
                $ctx = New-Object DirectoryServices.AccountManagement.PrincipalContext([DirectoryServices.AccountManagement.ContextType]::Machine)
            } else {
                $ctx = New-Object DirectoryServices.AccountManagement.PrincipalContext([DirectoryServices.AccountManagement.ContextType]::Domain, $me.Domain)
            }
            # the plain text exists only as this call's argument (ValidateCredentials takes a string)
            if ($ctx.ValidateCredentials($me.User, [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr))) {
                [Smt5Lsa]::Store('DefaultPassword', $bstr, $secure.Length)
                $ok = $true
            } else {
                [Console]::Out.WriteLine('Şifre yanlış / Wrong password (' + $try + '/3)')
            }
        } finally {
            if ($bstr -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
            $bstr = [IntPtr]::Zero
            if ($null -ne $ctx) { $ctx.Dispose() }
            if ($null -ne $secure) { $secure.Dispose() }
            $secure = $null
        }
    }
    if (-not $ok) { throw 'şifre 3 kez doğrulanamadı; otomatik oturum açılmadı / password not verified, automatic logon not enabled' }
    Set-ItemProperty -LiteralPath $Winlogon -Name 'AutoAdminLogon' -Value '1' -Type String
    Set-ItemProperty -LiteralPath $Winlogon -Name 'DefaultUserName' -Value $me.User -Type String
    Set-ItemProperty -LiteralPath $Winlogon -Name 'DefaultDomainName' -Value $me.Domain -Type String
    # a plain-text registry password would win over the LSA secret; a count would end autologon
    Remove-WinlogonValue 'DefaultPassword'
    Remove-WinlogonValue 'AutoLogonCount'
    # uninstall turns automatic logon off only when we turned it on
    New-Item -ItemType Directory -Force -Path $StateDir | Out-Null
    Write-TextFile $AutologonOurs ($me.Full + "`n")
    Write-Log ('otomatik oturum açıldı: ' + $me.Full)
    [Console]::Out.WriteLine('')
    [Console]::Out.WriteLine('Otomatik oturum açık. Bu pencereyi kapatabilirsiniz. / Automatic logon is on. You can close this window.')
}

function Disable-Autologon {
    Set-ItemProperty -LiteralPath $Winlogon -Name 'AutoAdminLogon' -Value '0' -Type String
    Remove-WinlogonValue 'DefaultPassword'
    Initialize-Lsa
    [Smt5Lsa]::Store('DefaultPassword', [IntPtr]::Zero, 0)
    Remove-PathSafe $AutologonOurs
    Write-Log 'otomatik oturum kapatıldı (AutoAdminLogon=0, LSA DefaultPassword silindi)'
}

# 'on' only when Windows logs on this user by itself.
function Get-AutologonStatus {
    $k = Get-ItemProperty -LiteralPath $Winlogon -ErrorAction SilentlyContinue
    if ($null -eq $k) { return 'off' }
    if (([string]$k.AutoAdminLogon -eq '1') -and ([string]$k.DefaultUserName -ieq (Get-CurrentUser).User)) { return 'on' }
    return 'off'
}
