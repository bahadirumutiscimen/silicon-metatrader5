//! Server mode: the system `ssh` to a Linux VPS. Targets, the argument sets
//! for remote commands and port-forwarding tunnels, host keys and errors.
//! Every non-interactive call uses BatchMode, so ssh never asks for a password.

use crate::exec::{Cmd, Runner};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `user@host[:port]`, `user@[v6]:port` or a `~/.ssh/config` alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

impl SshTarget {
    pub fn parse(s: &str) -> Result<SshTarget, String> {
        let s = s.trim();
        if s.is_empty() || s.starts_with('-') || s.chars().any(|c| c.is_whitespace() || c.is_control() || "'\"`$;&|<>\\".contains(c)) {
            return Err(format!("invalid SSH target: {s:?}"));
        }
        let (user, rest) = match s.rsplit_once('@') {
            Some((u, r)) if !u.is_empty() && !r.is_empty() => (Some(u.to_owned()), r),
            Some(_) => return Err(format!("invalid SSH target: {s:?}")),
            None => (None, s),
        };
        let port_of = |p: &str| -> Result<u16, String> {
            match p.parse::<u16>() {
                Ok(n) if n > 0 => Ok(n),
                _ => Err(format!("invalid SSH port: {p:?}")),
            }
        };
        let (host, port) = if let Some(r) = rest.strip_prefix('[') {
            let (h, tail) = r.split_once(']').ok_or_else(|| format!("invalid SSH target: {s:?}"))?;
            match tail {
                "" => (h.to_owned(), None),
                t => (h.to_owned(), Some(port_of(t.strip_prefix(':').ok_or_else(|| format!("invalid SSH target: {s:?}"))?)?)),
            }
        } else if rest.matches(':').count() == 1 {
            let (h, p) = rest.split_once(':').unwrap_or((rest, ""));
            (h.to_owned(), Some(port_of(p)?))
        } else {
            (rest.to_owned(), None)
        };
        if host.is_empty() || host.starts_with('-') {
            return Err(format!("invalid SSH target: {s:?}"));
        }
        Ok(SshTarget { user, host, port })
    }

    /// What ssh gets as destination: `user@host` or `host` (v6 without brackets).
    pub fn dest(&self) -> String {
        match &self.user {
            Some(u) => format!("{u}@{}", self.host),
            None => self.host.clone(),
        }
    }
}

/// Where the app keeps its ssh state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshOpts {
    /// `<config_dir>/ssh`: the app's own known_hosts, tunnel pid files.
    pub dir: PathBuf,
    /// Short directory for ControlMaster sockets (unix socket paths are
    /// limited to ~104 bytes, so not under "Application Support").
    pub control_dir: PathBuf,
    pub identity: Option<PathBuf>,
}

impl SshOpts {
    pub fn known_hosts(&self) -> PathBuf {
        self.dir.join("known_hosts")
    }
    pub fn control_path(&self) -> String {
        format!("{}/%C", self.control_dir.display())
    }
}

/// The server's own 127.0.0.1 ports (from its `.env`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemotePorts {
    pub bridge: u16,
    pub status: u16,
    pub display: u16,
}

impl Default for RemotePorts {
    fn default() -> Self {
        RemotePorts { bridge: 8001, status: 8002, display: 6081 }
    }
}

/// The local ports of one server: base+1 bridge, +2 status, +81 noVNC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortBlock(pub u16);

impl PortBlock {
    pub fn bridge(&self) -> u16 {
        self.0 + 1
    }
    pub fn status(&self) -> u16 {
        self.0 + 2
    }
    pub fn display(&self) -> u16 {
        self.0 + 81
    }
}

/// The first free block of 18000 + 100·n.
pub fn next_port_block(used: &[u16]) -> u16 {
    (0u16..).map(|n| 18000 + 100 * n).find(|b| !used.contains(b)).unwrap_or(18000)
}

/// An ssh `-o` value; values with spaces are double-quoted for ssh's parser.
fn opt(k: &str, v: &str) -> String {
    if v.contains(' ') {
        format!("{k}=\"{v}\"")
    } else {
        format!("{k}={v}")
    }
}

/// Options shared by every call: host keys, ControlMaster, port, identity.
fn common(t: &SshTarget, o: &SshOpts, batch: bool) -> Vec<String> {
    common_mux(t, o, batch, true)
}

/// `mux = false`: a connection of its own (the tunnel), never a ControlMaster.
fn common_mux(t: &SshTarget, o: &SshOpts, batch: bool, mux: bool) -> Vec<String> {
    common_flag(t, o, batch, mux, "-p")
}

/// `port_flag`: ssh and ssh-copy-id take `-p`, scp takes `-P`.
fn common_flag(t: &SshTarget, o: &SshOpts, batch: bool, mux: bool, port_flag: &str) -> Vec<String> {
    let mut a = Vec::new();
    let mut o_ = |k: &str, v: &str| {
        a.push("-o".to_owned());
        a.push(opt(k, v));
    };
    if batch {
        o_("BatchMode", "yes");
        o_("ConnectTimeout", "10");
    }
    o_("StrictHostKeyChecking", "yes");
    o_("UserKnownHostsFile", &o.known_hosts().display().to_string());
    if mux {
        o_("ControlMaster", "auto");
        o_("ControlPath", &o.control_path());
        o_("ControlPersist", "60");
    } else {
        o_("ControlMaster", "no");
        o_("ControlPath", "none");
    }
    if let Some(id) = &o.identity {
        o_("IdentitiesOnly", "yes");
        a.push("-i".to_owned());
        a.push(id.display().to_string());
    }
    if let Some(p) = t.port {
        a.push(port_flag.to_owned());
        a.push(p.to_string());
    }
    a
}

/// The shell a server runs scripts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    PowerShell,
}

/// Run `script` on the server with bash (login shell, so ~/.local/bin is on PATH).
pub fn exec_cmd(t: &SshTarget, o: &SshOpts, script: &str) -> Cmd {
    exec_shell(t, o, Shell::Bash, script)
}

/// Run `script` on the server with `shell`. PowerShell gets the script on
/// stdin (via `ps_launcher`), so no quoting layer touches it.
pub fn exec_shell(t: &SshTarget, o: &SshOpts, shell: Shell, script: &str) -> Cmd {
    let c = Cmd::new("ssh").args(common(t, o, true)).arg(t.dest());
    match shell {
        Shell::Bash => c.args(["bash", "-lc"]).arg(shell_quote(script)),
        Shell::PowerShell => c.arg(ps_launcher()).stdin_text(script),
    }
}

/// The remote command that runs a script read from stdin in PowerShell. It
/// is base64 only, so it survives both cmd.exe and PowerShell as the SSH
/// command of a Windows server.
pub fn ps_launcher() -> String {
    format!(
        "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}",
        ps_encode(PS_STDIN_RUNNER)
    )
}

const PS_STDIN_RUNNER: &str =
    "[Console]::InputEncoding=[Text.Encoding]::UTF8; [Console]::OutputEncoding=[Text.Encoding]::UTF8; $ProgressPreference='SilentlyContinue'; $s=[Console]::In.ReadToEnd(); & ([scriptblock]::Create($s))";

/// A line of a remote PowerShell's output as the job log should show it.
/// Non-interactive powershell.exe writes its progress and error streams to
/// stderr as CLIXML (`#< CLIXML` then `<Objs …>`): progress records are
/// dropped, error records become their plain text. Other lines pass as is.
pub fn ps_clixml_line(line: &str) -> Option<String> {
    let l = line.trim_start();
    if l.starts_with("#< CLIXML") {
        return None;
    }
    if !l.starts_with("<Objs ") {
        return Some(line.to_owned());
    }
    const OPEN: &str = "<S S=\"Error\">";
    let mut out = Vec::new();
    let mut rest = l;
    while let Some(i) = rest.find(OPEN) {
        rest = &rest[i + OPEN.len()..];
        let end = rest.find("</S>").unwrap_or(rest.len());
        let txt = rest[..end]
            .replace("_x000D__x000A_", " ")
            .replace("_x000A_", " ")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&");
        let txt = txt.trim();
        if !txt.is_empty() {
            out.push(txt.to_owned());
        }
        rest = &rest[end..];
    }
    (!out.is_empty()).then(|| out.join(" "))
}

/// `-EncodedCommand` payload: UTF-16LE, base64.
fn ps_encode(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for ch in bytes.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= ch.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Copy `local` to the server's home directory as `remote_name`, over the
/// same ControlMaster as the other calls.
pub fn scp_cmd(t: &SshTarget, o: &SshOpts, local: &Path, remote_name: &str) -> Cmd {
    Cmd::new("scp")
        .arg("-q")
        .args(common_flag(t, o, true, true, "-P"))
        .arg(local.display().to_string())
        .arg(format!("{}:{remote_name}", t.dest()))
}

/// The first line (at most 255 bytes) the SSH server sends on connect.
pub fn read_banner(host: &str, port: u16, timeout: Duration) -> Option<String> {
    use std::io::Read;
    use std::net::ToSocketAddrs;
    let addr = (host, port).to_socket_addrs().ok()?.next()?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, timeout).ok()?;
    s.set_read_timeout(Some(timeout)).ok()?;
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while line.len() < 255 {
        match s.read(&mut b) {
            Ok(1) if b[0] == b'\n' => break,
            Ok(1) => line.push(b[0]),
            _ => break,
        }
    }
    if line.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&line).trim_end_matches('\r').to_owned())
}

/// Does the server banner name Microsoft's OpenSSH port?
pub fn is_windows_banner(b: &str) -> bool {
    b.contains("OpenSSH_for_Windows")
}

/// The long-lived port-forwarding process of one server. It is its own
/// connection: joined to a ControlMaster, `ssh -N` would hand the forwards to
/// that (untracked) master and exit at once.
pub fn tunnel_cmd(t: &SshTarget, o: &SshOpts, local: PortBlock, remote: &RemotePorts) -> Cmd {
    let fwd = |l: u16, r: u16| format!("127.0.0.1:{l}:127.0.0.1:{r}");
    Cmd::new("ssh")
        .arg("-N")
        .args(common_mux(t, o, true, false))
        .args(["-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"])
        .args(["-L".to_owned(), fwd(local.bridge(), remote.bridge)])
        .args(["-L".to_owned(), fwd(local.status(), remote.status)])
        .args(["-L".to_owned(), fwd(local.display(), remote.display)])
        .arg(t.dest())
}

/// Retire the ControlMaster (stale, or after a group change): it takes no new
/// sessions, running ones (an install) finish; the next call opens a new one.
pub fn control_exit_cmd(t: &SshTarget, o: &SshOpts) -> Cmd {
    Cmd::new("ssh").args(common(t, o, true)).args(["-O", "stop"]).arg(t.dest())
}

/// A shell line for the user's Terminal: interactive (`-t`), no BatchMode, so
/// sudo / ssh-copy-id can ask for the password there.
pub fn terminal_ssh_line(t: &SshTarget, o: &SshOpts, script: &str) -> String {
    let mut parts = vec!["ssh".to_owned(), "-t".to_owned()];
    parts.extend(common(t, o, false).iter().map(|a| sh_word(a)));
    parts.push(sh_word(&t.dest()));
    parts.push(shell_quote(script));
    parts.join(" ")
}

/// [`terminal_ssh_line`] for a Windows server: `script` runs in PowerShell,
/// passed as `-EncodedCommand` (stdin stays the terminal, so `Read-Host` can
/// prompt there). Bypass: `smt5.ps1` is unsigned.
pub fn terminal_ps_line(t: &SshTarget, o: &SshOpts, script: &str) -> String {
    let mut parts = vec!["ssh".to_owned(), "-t".to_owned()];
    parts.extend(common(t, o, false).iter().map(|a| sh_word(a)));
    parts.push(sh_word(&t.dest()));
    parts.push(format!("powershell -NoProfile -ExecutionPolicy Bypass -EncodedCommand {}", ps_encode(script)));
    parts.join(" ")
}

/// The Terminal line that installs `pubkey` on the server (`ssh-copy-id`
/// passes `-o` values to ssh verbatim, so spaces stay double-quoted).
pub fn copy_id_line(t: &SshTarget, o: &SshOpts, pubkey: &std::path::Path) -> String {
    let mut l = format!(
        "ssh-copy-id -i {} -o {} -o StrictHostKeyChecking=yes",
        sh_word(&pubkey.display().to_string()),
        sh_word(&opt("UserKnownHostsFile", &o.known_hosts().display().to_string()))
    );
    if let Some(p) = t.port {
        l.push_str(&format!(" -p {p}"));
    }
    l.push(' ');
    l.push_str(&sh_word(&t.dest()));
    l
}

/// The Terminal line that installs `pubkey` (the key text) on a Windows
/// server: administrators use `administrators_authorized_keys` (with the ACL
/// sshd requires), others `~\\.ssh\\authorized_keys`. The key is appended as
/// UTF-8 without BOM (what sshd reads), on its own line even when the file
/// does not end with a newline; a key already there (any encoding the file
/// was written in, CRLF or LF) is not added again. The script travels as
/// `-EncodedCommand`; no BatchMode, so the password prompt works.
pub fn copy_id_line_windows(t: &SshTarget, o: &SshOpts, pubkey: &str) -> String {
    let key = pubkey.trim().replace('\'', "''");
    let script = format!(
        r#"$k='{key}'
$admin=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if($admin){{$f=Join-Path $env:ProgramData 'ssh\administrators_authorized_keys'}}else{{$f=Join-Path $HOME '.ssh\authorized_keys'}}
New-Item -ItemType Directory -Force -Path (Split-Path $f) | Out-Null
$t=''
if(Test-Path -LiteralPath $f){{$t=[IO.File]::ReadAllText($f)}}
$have=@($t -split "`n" | ForEach-Object {{ $_.Trim() }}) -contains $k
if(-not $have){{
$pre=''
if($t.Length -gt 0 -and -not $t.EndsWith("`n")){{$pre="`r`n"}}
[IO.File]::AppendAllText($f, ($pre + $k + "`r`n"), (New-Object Text.UTF8Encoding($false)))
}}
if($admin){{icacls $f /inheritance:r /grant "*S-1-5-32-544:F" /grant "*S-1-5-18:F" | Out-Null}}
"#
    );
    let mut parts = vec!["ssh".to_owned(), "-t".to_owned()];
    parts.extend(common(t, o, false).iter().map(|a| sh_word(a)));
    parts.push(sh_word(&t.dest()));
    parts.push(format!("powershell -NoProfile -EncodedCommand {}", ps_encode(&script)));
    parts.join(" ")
}

/// Open the user's terminal running `line` (macOS Terminal, or the Linux
/// default terminal emulator).
pub fn open_terminal_cmd(os: &str, line: &str) -> Cmd {
    if os == "macos" {
        let esc = line.replace('\\', "\\\\").replace('"', "\\\"");
        Cmd::new("osascript")
            .arg("-e")
            .arg(format!("tell application \"Terminal\" to do script \"{esc}\""))
            .args(["-e", "tell application \"Terminal\" to activate"])
    } else {
        Cmd::new("x-terminal-emulator")
            .args(["-e", "bash", "-c"])
            .arg(format!("{line}; echo; read -r -p 'Enter…' _"))
    }
}

/// `ssh -G` resolves an alias with the user's ssh config.
pub fn resolve_cmd(t: &SshTarget) -> Cmd {
    let mut c = Cmd::new("ssh").arg("-G");
    if let Some(p) = t.port {
        c = c.args(["-p".to_owned(), p.to_string()]);
    }
    c.arg(t.dest())
}

/// (hostname, port, user) from `ssh -G` output.
pub fn parse_ssh_g(out: &str) -> (String, u16, Option<String>) {
    let (mut host, mut port, mut user) = (String::new(), 22u16, None);
    for line in out.lines() {
        match line.split_once(' ') {
            Some(("hostname", v)) => host = v.trim().to_owned(),
            Some(("port", v)) => port = v.trim().parse().unwrap_or(22),
            Some(("user", v)) => user = Some(v.trim().to_owned()),
            _ => {}
        }
    }
    (host, port, user)
}

pub fn keyscan_cmd(host: &str, port: u16) -> Cmd {
    Cmd::new("ssh-keyscan").args(["-T".to_owned(), "5".to_owned(), "-p".to_owned(), port.to_string(), host.to_owned()])
}

/// One host key the server offered, with its fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HostKey {
    pub kind: String,
    pub sha256: String,
    /// The keyscan line, written to known_hosts once the user accepts it.
    #[serde(skip)]
    pub line: String,
}

/// Fingerprints of `ssh-keyscan` output (each line through `ssh-keygen -lf`).
pub fn host_keys(keyscan_out: &str, runner: &dyn Runner) -> Vec<HostKey> {
    let mut keys = Vec::new();
    for (i, line) in keyscan_out.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).enumerate() {
        let f = std::env::temp_dir().join(format!("smt5-hostkey-{}-{i}", std::process::id()));
        if std::fs::write(&f, format!("{line}\n")).is_err() {
            continue;
        }
        let out = runner.run(&Cmd::new("ssh-keygen").arg("-lf").arg(f.display().to_string()));
        let _ = std::fs::remove_file(&f);
        // "256 SHA256:abc… host (ED25519)"
        let o = out.stdout_trimmed();
        let sha = o.split_whitespace().nth(1).unwrap_or("");
        let kind = o.rsplit_once('(').map(|(_, k)| k.trim_end_matches(')')).unwrap_or("");
        if out.ok() && sha.starts_with("SHA256:") {
            keys.push(HostKey { kind: kind.to_owned(), sha256: sha.to_owned(), line: line.to_owned() });
        }
    }
    keys
}

/// Why an ssh call failed, from its stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshError {
    KeyRequired,
    HostKeyChanged,
    PortInUse,
    Network(String),
}

impl SshError {
    pub fn kind(&self) -> &'static str {
        match self {
            SshError::KeyRequired => "key_required",
            SshError::HostKeyChanged => "host_key_changed",
            SshError::PortInUse => "port_in_use",
            SshError::Network(_) => "network",
        }
    }
    pub fn detail(&self) -> String {
        match self {
            SshError::Network(d) => d.clone(),
            other => other.kind().to_owned(),
        }
    }
}

pub fn classify_ssh_error(stderr: &str) -> SshError {
    let s = stderr.to_ascii_lowercase();
    if s.contains("remote host identification has changed") || s.contains("host key verification failed") || s.contains("host key is known") {
        SshError::HostKeyChanged
    } else if s.contains("permission denied") || s.contains("too many authentication failures") {
        SshError::KeyRequired
    } else if s.contains("address already in use") || s.contains("cannot listen to port") {
        SshError::PortInUse
    } else {
        let last = stderr.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("ssh failed");
        SshError::Network(last.to_owned())
    }
}

/// `s` as one shell word, quoted only when it needs it (readable Terminal lines).
fn sh_word(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)) {
        s.to_owned()
    } else {
        shell_quote(s)
    }
}

/// POSIX single-quoting: the result is one shell word equal to `s`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn opts() -> SshOpts {
        SshOpts { dir: PathBuf::from("/cfg/ssh"), control_dir: PathBuf::from("/tmp/smt5-ssh-501"), identity: None }
    }

    #[test]
    fn parse_targets_and_resolve() {
        let t = SshTarget::parse("ubuntu@203.0.113.10").unwrap();
        assert_eq!((t.user.as_deref(), t.host.as_str(), t.port), (Some("ubuntu"), "203.0.113.10", None));
        assert_eq!(SshTarget::parse("box:2222").unwrap().port, Some(2222));
        let v6 = SshTarget::parse("u@[2001:db8::1]:2222").unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("2001:db8::1", Some(2222)));
        assert_eq!(v6.dest(), "u@2001:db8::1");
        let alias = SshTarget::parse("myvps").unwrap();
        assert_eq!((alias.user.as_deref(), alias.host.as_str(), alias.port), (None, "myvps", None));
        for bad in ["", "a b", "-oProxyCommand=x", "u@", "h:0", "h:99999", "u@[::1"] {
            assert!(SshTarget::parse(bad).is_err(), "{bad} must be rejected");
        }
        assert_eq!(parse_ssh_g("user ubuntu\nhostname 10.0.0.5\nport 2200\n"), ("10.0.0.5".to_owned(), 2200, Some("ubuntu".to_owned())));
        assert_eq!(resolve_cmd(&alias).display(), "ssh -G myvps");
    }

    #[test]
    fn common_args_are_safe() {
        let t = SshTarget::parse("u@h:2222").unwrap();
        let o = SshOpts { identity: Some(PathBuf::from("/k/id")), ..opts() };
        let d = exec_cmd(&t, &o, "true").display();
        for want in [
            "BatchMode=yes",
            "StrictHostKeyChecking=yes",
            "UserKnownHostsFile=/cfg/ssh/known_hosts",
            "ControlMaster=auto",
            "ControlPath=/tmp/smt5-ssh-501/%C",
            "ControlPersist=60",
            "-p 2222",
            "-i /k/id",
            "IdentitiesOnly=yes",
        ] {
            assert!(d.contains(want), "{want} missing in {d}");
        }
        let plain = exec_cmd(&SshTarget::parse("h").unwrap(), &opts(), "true").display();
        assert!(!plain.contains(" -p ") && !plain.contains(" -i "));
        assert!(plain.contains(" h bash -lc 'true'"), "{plain}");
    }

    #[test]
    fn tunnel_forwards_loopback_only() {
        let t = SshTarget::parse("h").unwrap();
        let c = tunnel_cmd(&t, &opts(), PortBlock(18100), &RemotePorts { bridge: 8001, status: 8002, display: 6081 });
        let d = c.display();
        for want in [
            " -N ",
            "ExitOnForwardFailure=yes",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=3",
            "-L 127.0.0.1:18101:127.0.0.1:8001",
            "-L 127.0.0.1:18102:127.0.0.1:8002",
            "-L 127.0.0.1:18181:127.0.0.1:6081",
        ] {
            assert!(d.contains(want), "{want} missing in {d}");
        }
        // its own connection: a ControlMaster would take over the forwards and the
        // `ssh -N` child would exit at once (Final review C1)
        for want in ["ControlMaster=no", "ControlPath=none"] {
            assert!(d.contains(want), "{want} missing in {d}");
        }
        assert!(!d.contains("ControlPersist") && !d.contains("ControlMaster=auto"), "{d}");
        assert!(d.contains("BatchMode=yes") && d.contains("StrictHostKeyChecking=yes"));
        // `-O stop`: no new mux clients, running sessions (an install) keep going
        let x = control_exit_cmd(&t, &opts()).display();
        assert!(x.contains(" -O stop "), "{x}");
    }

    #[test]
    fn copy_id_line_keeps_paths_with_spaces() {
        let t = SshTarget::parse("u@h:2200").unwrap();
        let o = SshOpts { dir: PathBuf::from("/Users/u/Library/Application Support/S/ssh"), ..opts() };
        let l = copy_id_line(&t, &o, std::path::Path::new("/Users/u/.ssh/siliconmt5_ed25519.pub"));
        assert!(l.starts_with("ssh-copy-id -i /Users/u/.ssh/siliconmt5_ed25519.pub "), "{l}");
        assert!(l.contains(r#"'UserKnownHostsFile="/Users/u/Library/Application Support/S/ssh/known_hosts"'"#), "{l}");
        assert!(l.contains("-p 2200") && l.ends_with(" u@h") && l.contains("StrictHostKeyChecking=yes"), "{l}");
    }

    #[test]
    fn port_blocks() {
        assert_eq!(next_port_block(&[]), 18000);
        assert_eq!(next_port_block(&[18000, 18200]), 18100);
        let b = PortBlock(18100);
        assert_eq!((b.bridge(), b.status(), b.display()), (18101, 18102, 18181));
        assert_eq!(RemotePorts::default(), RemotePorts { bridge: 8001, status: 8002, display: 6081 });
    }

    #[test]
    fn shell_quote_roundtrip() {
        for s in ["plain", "a b", "it's", "$HOME", "\"q\"", "x;rm -rf /", "", "`id`"] {
            let o = std::process::Command::new("bash").arg("-c").arg(format!("printf %s {}", shell_quote(s))).output().unwrap();
            assert_eq!(String::from_utf8_lossy(&o.stdout), s);
        }
    }

    #[test]
    fn classify() {
        assert_eq!(classify_ssh_error("u@h: Permission denied (publickey)."), SshError::KeyRequired);
        assert_eq!(classify_ssh_error("@@@\nWARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!\n"), SshError::HostKeyChanged);
        assert_eq!(classify_ssh_error("Host key verification failed."), SshError::HostKeyChanged);
        assert_eq!(classify_ssh_error("No ED25519 host key is known for h and you have requested strict checking."), SshError::HostKeyChanged);
        assert_eq!(classify_ssh_error("bind [127.0.0.1]:18001: Address already in use\nchannel_setup_fwd_listener_tcpip: cannot listen to port: 18001"), SshError::PortInUse);
        assert_eq!(
            classify_ssh_error("ssh: connect to host x port 22: Connection timed out\n"),
            SshError::Network("ssh: connect to host x port 22: Connection timed out".to_owned())
        );
        assert_eq!(SshError::KeyRequired.kind(), "key_required");
    }

    #[test]
    fn open_terminal_escapes() {
        let line = r#"ssh -t 'u@h' 'echo "hi" \x'"#;
        let mac = open_terminal_cmd("macos", line);
        assert_eq!(mac.program, "osascript");
        assert!(mac.args.iter().any(|a| a.contains(r#"do script "ssh -t 'u@h' 'echo \"hi\" \\x'""#)), "{:?}", mac.args);
        let lin = open_terminal_cmd("linux", line);
        assert_eq!(lin.program, "x-terminal-emulator");
        assert!(lin.args.last().unwrap().starts_with(line));
        let t = SshTarget::parse("u@h:2200").unwrap();
        let l = terminal_ssh_line(&t, &opts(), "sudo true");
        assert!(l.starts_with("ssh -t ") && l.contains("-p 2200") && l.ends_with("'sudo true'") && !l.contains("BatchMode"), "{l}");
        // paths with spaces (macOS "Application Support") stay one ssh option value
        let sp = SshOpts { dir: PathBuf::from("/Users/u/Library/Application Support/S/ssh"), ..opts() };
        let c = exec_cmd(&t, &sp, "true");
        assert!(c.args.iter().any(|a| a == "UserKnownHostsFile=\"/Users/u/Library/Application Support/S/ssh/known_hosts\""), "{:?}", c.args);
    }

    #[test]
    fn host_key_lines_are_fingerprinted() {
        let scan = "# h:22 SSH-2.0-OpenSSH\nh ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n";
        let r = crate::exec::MockRunner::new().on("ssh-keygen -lf", 0, "256 SHA256:abcDEF h (ED25519)\n");
        let keys = host_keys(scan, &r);
        assert_eq!(keys.len(), 1);
        assert_eq!((keys[0].kind.as_str(), keys[0].sha256.as_str()), ("ED25519", "SHA256:abcDEF"));
        assert!(keys[0].line.starts_with("h ssh-ed25519 "));
        assert_eq!(keyscan_cmd("h", 2200).display(), "ssh-keyscan -T 5 -p 2200 h");
    }

    /// Minimal base64 decoder + UTF-16LE for the tests.
    fn decode_ps(b64: &str) -> String {
        let val = |c: u8| match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            _ => 63,
        };
        let mut bytes = Vec::new();
        let mut acc = 0u32;
        let mut bits = 0;
        for c in b64.bytes().filter(|&c| c != b'=') {
            acc = (acc << 6) | val(c) as u32;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                bytes.push((acc >> bits) as u8);
                acc &= (1 << bits) - 1;
            }
        }
        let u: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&u).unwrap()
    }

    #[test]
    fn ps_launcher_is_base64_only() {
        let l = ps_launcher();
        assert!(l.starts_with("powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand "), "{l}");
        let b64 = l.rsplit(' ').next().unwrap();
        assert!(
            !b64.is_empty() && b64.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'=')
        );
        let s = decode_ps(b64);
        assert!(s.contains("ReadToEnd()") && s.contains("OutputEncoding"), "{s}");
        let (inp, read) = (s.find("InputEncoding=[Text.Encoding]::UTF8"), s.find("ReadToEnd()"));
        assert!(inp.is_some() && inp < read, "stdin must be UTF-8 before the first read: {s}");
    }

    #[test]
    fn exec_shell_powershell_feeds_script_on_stdin() {
        let t = SshTarget::parse("u@203.0.113.10").unwrap();
        let c = exec_shell(&t, &opts(), Shell::PowerShell, "Write-Output ş");
        assert_eq!(c.stdin_text.as_deref(), Some("Write-Output ş"));
        let d = c.display();
        assert!(d.contains("BatchMode=yes") && d.contains("ControlPath"), "{d}");
        assert!(!d.contains("bash"), "{d}");
        let b = exec_shell(&t, &opts(), Shell::Bash, "echo hi");
        assert_eq!(b, exec_cmd(&t, &opts(), "echo hi"));
        assert!(b.stdin_text.is_none());
    }

    #[test]
    fn scp_cmd_uses_capital_p_port() {
        let t = SshTarget::parse("u@203.0.113.10:2222").unwrap();
        let c = scp_cmd(&t, &opts(), std::path::Path::new("/tmp/rt.tgz"), "smt5-runtime.tgz");
        assert_eq!(c.program, "scp");
        let d = c.display();
        assert!(d.contains(" -P 2222 ") && !d.contains(" -p "), "{d}");
        assert!(d.contains("ControlPath"), "{d}");
        assert_eq!(c.args.last().unwrap(), "u@203.0.113.10:smt5-runtime.tgz");
    }

    #[test]
    fn windows_banner() {
        assert!(is_windows_banner("SSH-2.0-OpenSSH_for_Windows_9.5"));
        assert!(!is_windows_banner("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3"));
    }

    #[test]
    fn read_banner_reads_first_line() {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            s.write_all(b"SSH-2.0-X\r\nrest\n").unwrap();
        });
        let b = read_banner("127.0.0.1", port, std::time::Duration::from_secs(2));
        h.join().unwrap();
        assert_eq!(b.as_deref(), Some("SSH-2.0-X"));
        // closed port
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert_eq!(read_banner("127.0.0.1", port, std::time::Duration::from_millis(500)), None);
    }

    #[test]
    fn terminal_ps_line_runs_encoded_script_interactively() {
        let t = SshTarget::parse("u@203.0.113.10:2200").unwrap();
        let script = "& \"$env:LOCALAPPDATA\\x\\smt5.ps1\" autologon";
        let l = terminal_ps_line(&t, &opts(), script);
        assert!(l.starts_with("ssh -t ") && l.contains("-p 2200") && l.contains(" u@203.0.113.10 "), "{l}");
        assert!(!l.contains("BatchMode") && !l.contains("NonInteractive"), "Read-Host must be able to prompt: {l}");
        assert!(l.contains(" powershell -NoProfile -ExecutionPolicy Bypass -EncodedCommand "), "{l}");
        let b64 = l.rsplit(' ').next().unwrap();
        assert!(b64.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'='), "{b64}");
        assert_eq!(decode_ps(b64), script);
    }

    #[test]
    fn copy_id_windows_line() {
        let t = SshTarget::parse("u@203.0.113.10").unwrap();
        let l = copy_id_line_windows(&t, &opts(), "ssh-ed25519 AAAA it's@example.net");
        assert!(l.starts_with("ssh -t "), "{l}");
        assert!(!l.contains("BatchMode"), "{l}");
        let b64 = l.rsplit(' ').next().unwrap();
        assert!(l.contains(" powershell -NoProfile -EncodedCommand "), "{l}");
        let s = decode_ps(b64);
        assert!(s.contains("ssh-ed25519 AAAA it''s@example.net"), "{s}");
        assert!(s.contains("administrators_authorized_keys") && s.contains("S-1-5-18"), "{s}");
        // appended as UTF-8 without BOM, on a new line even when the file has no final newline
        assert!(s.contains("[IO.File]::AppendAllText($f,") && s.contains("New-Object Text.UTF8Encoding($false)"), "{s}");
        assert!(!s.contains("Add-Content") && !s.contains("-Encoding ascii"), "{s}");
        assert!(s.contains("-not $t.EndsWith(\"`n\")") && s.contains("\"`r`n\""), "{s}");
        // the duplicate check reads the file whatever its encoding (BOM) and ignores CR / spaces
        assert!(s.contains("[IO.File]::ReadAllText($f)") && s.contains(".Trim()") && s.contains("-contains $k"), "{s}");
    }

    #[test]
    fn clixml_progress_dropped_errors_kept() {
        assert_eq!(ps_clixml_line("#< CLIXML"), None);
        assert_eq!(ps_clixml_line("Kurulum tamam."), Some("Kurulum tamam.".to_owned()));
        let prog = r#"<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04"><Obj S="progress" RefId="0"><MS><PR N="Record"><AV>Preparing modules for first use.</AV></PR></MS></Obj></Objs>"#;
        assert_eq!(ps_clixml_line(prog), None);
        let err = r#"<Objs Version="1.1.0.1"><S S="Error">Get-Content : Cannot find path &apos;C:\x&apos; because it does not exist._x000D__x000A_</S><S S="Error">At line:1 char:1_x000D__x000A_</S></Objs>"#;
        assert_eq!(ps_clixml_line(err).as_deref(), Some(r"Get-Content : Cannot find path 'C:\x' because it does not exist. At line:1 char:1"));
        assert!(ps_launcher().len() < 8000);
    }
}
