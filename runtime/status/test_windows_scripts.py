"""Static (text) checks of the Windows server scripts in runtime/windows.

PowerShell does not run here (no pwsh on Linux CI or the dev machines), so
these tests read the .ps1 files as text: every command is dispatched, no
password is written in plain text, everything listens on 127.0.0.1, paths
are quoted, downloads are pinned and verified.
"""
import json
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
RUNTIME = os.path.normpath(os.path.join(HERE, ".."))
WIN = os.path.join(RUNTIME, "windows")
SMT5 = os.path.join(WIN, "smt5.ps1")
LIBS = ["common.ps1", "download.ps1", "autologon.ps1", "tasks.ps1", "settings.ps1", "install.ps1"]
COMMANDS = ["install", "start", "stop", "restart", "reset", "uninstall", "autologon",
            "autologon-status", "session", "reboot", "stats"]
BOM = b"\xef\xbb\xbf"


def read(path):
    with open(path, "rb") as fh:
        return fh.read().decode("utf-8-sig")


def ps1_files():
    out = [SMT5] + [os.path.join(WIN, "lib", n) for n in LIBS]
    return out


def code_lines(text):
    """Lines without full-line comments and without <# ... #> blocks."""
    text = re.sub(r"<#.*?#>", "", text, flags=re.S)
    return [l for l in text.splitlines() if not l.lstrip().startswith("#")]


def fn(text, name):
    """The body of `function <name> ... {` up to its closing brace at column 0."""
    m = re.search(r"(?m)^function " + re.escape(name) + r"\b[^\n]*\{\n(.*?)\n\}", text, re.S)
    assert m, name
    return m.group(1)


def all_code():
    return "\n".join("\n".join(code_lines(read(p))) for p in ps1_files())


class WindowsScriptsTest(unittest.TestCase):
    def test_files_exist(self):
        for p in ps1_files() + [os.path.join(WIN, "versions.json"),
                                os.path.join(RUNTIME, "config", "requirements-windows.txt")]:
            self.assertTrue(os.path.isfile(p), p)

    def test_all_commands_dispatched(self):
        text = read(SMT5)
        m = re.search(r"switch\s*\(\s*\$Command\s*\)\s*\{(.*)\n\s*default\s*\{", text, re.S)
        self.assertIsNotNone(m, "switch ($Command) { ... default { ... } } in smt5.ps1")
        body = m.group(1)
        for c in COMMANDS:
            self.assertRegex(body, r"(?m)^\s*'" + re.escape(c) + r"'\s*\{", c)

    def test_libs_dot_sourced(self):
        text = read(SMT5)
        for n in LIBS:
            self.assertIn("'" + n + "'", text, n)

    def test_no_plaintext_password(self):
        for p in ps1_files():
            for line in code_lines(read(p)):
                self.assertNotRegex(line, r"(Set|New)-ItemProperty.*DefaultPassword", p)
                self.assertNotIn("ConvertFrom-SecureString", line, p)
                self.assertNotRegex(line, r"(?i)Write-Output\s+\$pw", p)
                self.assertNotRegex(line, r"(?i)Add-Content.*pass", p)
                self.assertNotRegex(line, r"(?i)Write-Log.*\$(pw|password|secure|bstr|token)\b", p)

    def test_autologon_uses_lsa_and_clears_secret(self):
        text = read(os.path.join(WIN, "lib", "autologon.ps1"))
        for needle in ["Read-Host", "-AsSecureString", "PrincipalContext", "ValidateCredentials",
                       "LsaOpenPolicy", "LsaStorePrivateData", "'DefaultPassword'", "ZeroFreeBSTR",
                       "finally", "AutoAdminLogon", "DefaultUserName", "DefaultDomainName",
                       "Remove-ItemProperty"]:
            self.assertIn(needle, text, needle)
        # off: the LSA secret is deleted (null data) and AutoAdminLogon=0
        self.assertRegex(text, r"\[IntPtr\]::Zero")
        self.assertRegex(text, r"AutoAdminLogon'?\s+-Value\s+'0'")
        # ZeroFreeBSTR sits in a finally block
        self.assertRegex(text, r"finally\s*\{[^}]*ZeroFreeBSTR", "BSTR freed in finally")

    def test_loopback_only(self):
        code = all_code()
        self.assertIn("VALUE_OF_LOOPBACKONLY=1", code)
        self.assertIn("SET_LOOPBACKONLY=1", code)
        self.assertIn("SERVER_ADD_FIREWALL_EXCEPTION=0", code)
        self.assertIn("VALUE_OF_ACCEPTHTTPCONNECTIONS=0", code)
        self.assertNotIn("0.0.0.0", code)
        self.assertNotRegex(code, r"(?i)New-NetFirewallRule|netsh\s+advfirewall")
        # websockify (display.py, the Display task) binds 127.0.0.1 too
        self.assertIn("' 127.0.0.1:' + $port + ' 127.0.0.1:' + ($port + 1000)", code)
        self.assertIn(r"^127\.0\.0\.1:\d{1,5}$", read(os.path.join(WIN, "display.py")))

    def test_paths_quoted(self):
        # an unquoted $Root\x / $env:LOCALAPPDATA\x breaks on profiles with spaces
        pat = re.compile(r"\$(Root|env:LOCALAPPDATA|Rt|PyDir|Mt5Dir|StateDir|LogDir|NovncDir)\\")
        for p in ps1_files():
            for n, line in enumerate(code_lines(read(p)), 1):
                for m in pat.finditer(line):
                    before = line[:m.start()]
                    self.assertEqual(before.count('"') % 2, 1, f"{p}:{n}: unquoted path: {line.strip()}")

    def test_ascii_or_bom(self):
        # Windows PowerShell 5.1 reads a .ps1 without BOM as the ANSI code page
        for p in ps1_files():
            with open(p, "rb") as fh:
                raw = fh.read()
            if any(b > 127 for b in raw):
                self.assertTrue(raw.startswith(BOM), f"{p}: non-ASCII text needs a UTF-8 BOM")

    def test_ps51_syntax_only(self):
        code = all_code()
        self.assertNotRegex(code, r"\?\?", "no null-coalescing (PS 7)")
        self.assertNotRegex(code, r"\?\.", "no null-conditional (PS 7)")
        self.assertNotIn("-Parallel", code)
        self.assertNotRegex(code, r"\$\w+\s*\?\s+[^:]+\s+:\s", "no ternary (PS 7)")

    def test_error_handling(self):
        text = read(SMT5)
        self.assertIn("$ErrorActionPreference = 'Stop'", text)
        self.assertIn("[Console]::Error.WriteLine(", text)
        # the top-level catch prints to stderr and exits 1; $LASTEXITCODE of the
        # last native call must not leak out: explicit exit 0 at the end
        self.assertRegex(text.rstrip(), re.compile(r"\}\s*catch\s*\{(?:(?!\n\}).)*\[Console\]::Error\.WriteLine\((?:(?!\n\}).)*exit 1\s*\n\}\s*exit 0$", re.S))
        self.assertIn("smt5-ps.log", all_code())

    def test_invariant_numbers(self):
        code = all_code()
        self.assertIn("SMT5STATS win", code)
        self.assertIn("InvariantCulture", code)

    def test_outputs(self):
        code = all_code()
        for needle in ["'on'", "'off'", "session=", " task=", "'rebooting'",
                       "Windows oturumu açık değil; yeniden başlatma ya da giriş bekleniyor"]:
            self.assertIn(needle, code, needle)

    def test_stop_contract(self):
        code = all_code()
        for needle in ["stop.request", "pids.json", "taskkill.exe", "/T", "/F", "/PID",
                       "SiliconMT5-Supervisor", "/End", "20"]:
            self.assertIn(needle, code, needle)

    def test_tasks(self):
        text = read(os.path.join(WIN, "lib", "tasks.ps1"))
        for needle in ["SiliconMT5-Supervisor", "SiliconMT5-Console", "-AtLogOn", "Interactive",
                       "pythonw.exe", "supervisor.py", "-RestartCount 3", "Microsoft-Windows-TerminalServices-LocalSessionManager/Operational",
                       "EventID=24", "tscon", "/dest:console", "quser", "S-1-5-18"]:
            self.assertIn(needle, text, needle)

    def test_install_steps(self):
        code = all_code()
        for needle in ["InstallAllUsers=0", "PrependPath=0", "Include_launcher=0", "Shortcuts=0",
                       "Include_test=0", "TargetDir=", "requirements-windows.txt", ".siliconmt5",
                       "foreign_mt5", "/auto", "tightvnc-preexisting", "prev-settings.json",
                       "ADDLOCAL=Server", "SERVER_REGISTER_AS_SERVICE=1", "SET_PASSWORD=1",
                       "SET_RFBPORT=1", "VALUE_OF_RFBPORT=", "1000", "/norestart", "vnc.html"]:
            self.assertIn(needle, code, needle)

    def test_display_task(self):
        """websockify runs in its own task (SYSTEM, at startup): the screen
        (TightVNC shows the logon screen) works without a user session."""
        tasks = read(os.path.join(WIN, "lib", "tasks.ps1"))
        common = read(os.path.join(WIN, "lib", "common.ps1"))
        smt5 = read(SMT5)
        self.assertIn("$DisplayTask = 'SiliconMT5-Display'", common)
        reg = fn(tasks, "Register-DisplayTask")
        for needle in ["-AtStartup", "'S-1-5-18'", "ServiceAccount", "pythonw.exe", "Get-DisplayArguments", "-Force"]:
            self.assertIn(needle, reg, needle)
        args = fn(tasks, "Get-DisplayArguments")
        for needle in ["display.py", "--web", "$NovncDir", "SMT5_DISPLAY_PORT", "6081", "--log-max-mb", "SMT5_LOG_MAX_MB"]:
            self.assertIn(needle, args, needle)
        self.assertLess(args.index("--log-max-mb"), args.index("--web"))
        # stop leaves the screen running; uninstall ends and deletes the task
        stop = fn(tasks, "Stop-Smt5")
        self.assertNotIn("DisplayTask", stop)
        self.assertIn("Get-StrayProcesses -ExceptDisplay", stop)
        self.assertNotRegex(stop, r"Get-StrayProcesses(?! -ExceptDisplay)")
        un = fn(tasks, "Unregister-Smt5Tasks")
        self.assertIn("Stop-DisplayTask", un)
        self.assertIn("$DisplayTask", un)
        self.assertLess(un.index("Stop-DisplayTask"), un.index("Unregister-ScheduledTask"))
        sd = fn(tasks, "Stop-DisplayTask")
        self.assertIn("taskkill.exe", sd)
        self.assertLess(sd.index("taskkill.exe"), sd.index("/End"), "the launcher tree goes before the task ends")
        # install stops it before Python/pip are touched, registers and starts it at the end
        inst = fn(smt5, "Install-Smt5")
        self.assertLess(inst.index("Stop-DisplayTask"), inst.index("Install-Python"))
        self.assertLess(inst.index("Register-Smt5Tasks"), inst.index("Start-DisplayTask"))
        self.assertIn("Register-DisplayTask", fn(tasks, "Register-Smt5Tasks"))
        # start/restart: re-registered when the port changed, started when not running
        sync = fn(tasks, "Sync-DisplayTask")
        self.assertIn(".Arguments", sync)
        self.assertIn("Register-DisplayTask", sync)
        self.assertIn("Start-DisplayTask", sync)
        start = fn(tasks, "Start-Smt5")
        self.assertLess(start.index("Sync-DisplayTask"), start.index("Test-UserSession"))
        # display processes (launcher + descendants) are told apart by state\display.json
        self.assertIn("$DisplayFile = Join-Path $StateDir 'display.json'", common)
        tree = fn(common, "Get-DisplayTreeIds")
        self.assertIn("ParentProcessId", tree)
        self.assertIn("CreationDate", tree)
        self.assertRegex(smt5, r"\$keep = @\([^)]*'display\.json'")

    def test_installed_marker(self):
        """state\\installed (the VERSION digest) is written only at the end of a
        successful install; reset keeps it, the app reads INSTALLED=yes|no."""
        smt5 = read(SMT5)
        common = read(os.path.join(WIN, "lib", "common.ps1"))
        self.assertIn("$InstalledFile = Join-Path $StateDir 'installed'", common)
        inst = fn(smt5, "Install-Smt5")
        i = inst.index("Write-TextFile $InstalledFile")
        for step in ["Install-Python", "Install-PipPackages", "Install-Mt5", "Install-TightVnc", "Install-Novnc",
                     "Register-Smt5Tasks", "Set-SessionSettings", "Start-DisplayTask"]:
            self.assertLess(inst.index(step), i, step)
        self.assertIn("$VersionFile", inst)
        self.assertRegex(smt5, r"\$keep = @\([^)]*'installed'")

    def test_installer_log(self):
        """The Logs page reads logs\\installer.log: install tees its lines there."""
        common = read(os.path.join(WIN, "lib", "common.ps1"))
        smt5 = read(SMT5)
        self.assertIn("$script:InstallerLog = Join-Path $LogDir 'installer.log'", fn(smt5, "Install-Smt5"))
        info = fn(common, "Write-Info")
        self.assertIn("$script:InstallerLog", info)
        self.assertIn("$script:InstallerLog", smt5[smt5.index("} catch {"):])

    def test_mt5_install_progress(self):
        """Up to 15 min of waiting prints a line every 30 s (keeps the job stream alive)."""
        inst = fn(read(os.path.join(WIN, "lib", "install.ps1")), "Install-Mt5")
        self.assertRegex(inst, r"\$i % 3 -eq 0\) \{ Write-Info")
        self.assertRegex(inst, r"\$i % 6 -eq 0\) \{ Write-Info")
        # mt5cfg.ini belongs to supervisor.py (written from .env before MT5 starts)
        self.assertNotIn("Copy-Mt5Config", read(os.path.join(WIN, "lib", "install.ps1")))

    def test_reset_and_uninstall(self):
        code = all_code()
        for needle in ["'Config'", "'bases'", "'logs'", "'Files'", "'Tester'", "msiexec.exe", "/x"]:
            self.assertIn(needle, code, needle)

    def test_ownership_gates(self):
        """Fail closed: without our own marker nothing is stopped or removed."""
        lib = lambda n: read(os.path.join(WIN, "lib", n))
        inst, common, auto, smt5 = lib("install.ps1"), lib("common.ps1"), lib("autologon.ps1"), read(SMT5)
        # TightVNC: uninstall only with the positive marker written after our MSI ran
        un = re.search(r"function Uninstall-TightVnc \{(.*?)\n\}", inst, re.S).group(1)
        self.assertRegex(un, re.compile(r"if \(-not \(Test-Path -LiteralPath \$TvnOurs\)\)\s*\{(?:(?!\n    \}).)*\n        return\n    \}", re.S), "tightvnc-ours gate")
        self.assertLess(un.index("$TvnOurs"), un.index("msiexec.exe"))
        self.assertIn("$TvnOurs = Join-Path $StateDir 'tightvnc-ours'", inst)
        # MT5 folder counts as ours only with our marker (or our install in progress)
        top = re.search(r"function Test-OurPath\(\[string\]\$Path\) \{(.*?)\n\}", common, re.S).group(1)
        self.assertIn("$dirs = @($PyDir)", top)
        self.assertRegex(top, r"if \(\(Test-Path -LiteralPath \$Mt5Marker\) -or \(Test-Path -LiteralPath \$Mt5Installing\)\) \{ \$dirs \+= \$Mt5Dir \}")
        self.assertNotIn("@($PyDir, $Mt5Dir)", common)
        # stats: every process but tvnserver must be ours (terminal64 too)
        self.assertIn("if (($p.ProcessName -ne 'tvnserver') -and -not (Test-OurPath (Get-ProcPath $p))) { continue }", smt5)
        # Apps & features entry: the folder itself, not "MetaTrader 5 <Broker>"
        self.assertIn("$prefix = $Mt5Dir.TrimEnd('\\') + '\\'", inst)
        self.assertIn("$us.IndexOf($prefix, [StringComparison]::OrdinalIgnoreCase)", inst)
        self.assertNotIn("IndexOf($Mt5Dir", inst)
        # autologon: marker written on enable, removed on off, kept by reset, checked by uninstall
        self.assertIn("$AutologonOurs = Join-Path $StateDir 'autologon-ours'", common)
        en = re.search(r"function Enable-Autologon \{(.*?)\n\}", auto, re.S).group(1)
        self.assertIn("Write-TextFile $AutologonOurs", en)
        dis = re.search(r"function Disable-Autologon \{(.*?)\n\}", auto, re.S).group(1)
        self.assertIn("Remove-PathSafe $AutologonOurs", dis)
        self.assertRegex(smt5, r"\$keep = @\([^)]*'autologon-ours'")
        self.assertRegex(smt5, r"'otomatik oturum' = \{\s*if \(Test-Path -LiteralPath \$AutologonOurs\) \{ Disable-Autologon \}")
        # explicit `autologon off` stays unconditional
        self.assertRegex(smt5, r"if \(\$Arg -eq 'off'\) \{ Assert-Admin; Disable-Autologon;")

    def test_empty_password_rejected(self):
        text = read(os.path.join(WIN, "lib", "autologon.ps1"))
        i = text.index("if ($secure.Length -eq 0)")
        self.assertLess(i, text.index("ValidateCredentials("), "checked before ValidateCredentials")
        self.assertRegex(text[i:i + 200], r"continue", "counts as a failed attempt")

    def test_download_verified(self):
        text = read(os.path.join(WIN, "lib", "download.ps1"))
        for needle in ["function Get-Pinned", "versions.json", "Invoke-WebRequest", "Tls12",
                       "Get-FileHash", "SHA256", "sha256 uyuşmuyor", "Get-AuthenticodeSignature",
                       "'Valid'", "SignerCertificate.Subject", "-UseBasicParsing"]:
            self.assertIn(needle, text, needle)

    def test_versions_pinned(self):
        with open(os.path.join(WIN, "versions.json")) as fh:
            v = json.load(fh)
        self.assertEqual(set(v), {"python", "tightvnc", "novnc", "mt5"})
        for name, e in v.items():
            self.assertTrue(e["url"].startswith("https://"), name)
        for name in ("python", "novnc"):
            self.assertRegex(v[name]["sha256"], r"^[0-9a-f]{64}$", name)
        for name in ("tightvnc", "mt5"):
            self.assertTrue(v[name].get("signer"), name)
        self.assertEqual(v["python"]["version"], "3.13.17")
        self.assertIn("python-3.13.17-amd64.exe", v["python"]["url"])
        self.assertEqual(v["tightvnc"]["version"], "2.8.90")
        self.assertTrue(v["tightvnc"]["url"].endswith("-64bit.msi"))
        self.assertEqual(v["novnc"]["version"], "1.7.0")
        self.assertIn("v1.7.0.zip", v["novnc"]["url"])

    def test_requirements_windows(self):
        with open(os.path.join(RUNTIME, "config", "requirements-windows.txt")) as fh:
            pins = [l.strip() for l in fh if l.strip() and not l.startswith("#")]
        self.assertEqual(pins, ["MetaTrader5==5.0.6231", "siliconmetatrader5==1.2.3", "rpyc==6.0.2",
                                "plumbum==1.10.0", "websockify==0.13.0"])


if __name__ == "__main__":
    unittest.main()
