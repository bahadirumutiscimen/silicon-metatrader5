//! Server mode records: the VPSes the app manages (saved in the app config)
//! and the app's own known_hosts for them.

use serde::{Deserialize, Serialize};
use smt5_core::exec::Cmd;
use smt5_core::remote::{exec_shell, is_windows_banner, next_port_block, HostKey, Shell, SshOpts, SshTarget};
use smt5_core::remote_setup::ServerMode;
use smt5_core::util::random_hex;
use std::path::{Path, PathBuf};

use crate::backend::AppConfig;
use crate::lf;

/// One VPS the app manages. No password or token is ever stored here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    /// `user@host[:port]` or a `~/.ssh/config` alias.
    pub ssh: String,
    pub identity: Option<PathBuf>,
    /// "linux-native", "docker" or "windows".
    pub mode: String,
    /// Local ports: +1 bridge, +2 status API, +81 noVNC (fixed once chosen).
    pub port_base: u16,
    /// The accepted host key fingerprint (SHA256:…).
    pub host_key: String,
    pub added_at: i64,
}

impl ServerConfig {
    pub fn target(&self) -> Result<SshTarget, String> {
        SshTarget::parse(&self.ssh)
    }
}

pub fn find<'a>(cfg: &'a AppConfig, id: &str) -> Option<&'a ServerConfig> {
    cfg.servers.iter().find(|s| s.id == id)
}

/// A new record with the next free port block (not saved yet).
pub fn new_server(cfg: &AppConfig, name: &str, ssh: &str, identity: Option<PathBuf>, host_key: &str) -> Result<ServerConfig, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err(lf!("sunucu adı 1–40 karakter olmalı", "the server name must be 1–40 characters"));
    }
    let t = SshTarget::parse(ssh).map_err(|_| lf!("geçersiz SSH hedefi: {}", "invalid SSH target: {}", ssh))?;
    let ssh = ssh.trim().to_owned();
    if cfg.servers.iter().any(|s| s.ssh == ssh) {
        return Err(lf!("{} zaten ekli", "{} is already added", t.dest()));
    }
    let used: Vec<u16> = cfg.servers.iter().map(|s| s.port_base).collect();
    let mut id = random_hex(3).map_err(|e| e.to_string())?;
    while find(cfg, &id).is_some() {
        id = random_hex(3).map_err(|e| e.to_string())?;
    }
    let added_at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    Ok(ServerConfig {
        id,
        name: name.to_owned(),
        ssh,
        identity,
        mode: "linux-native".to_owned(),
        port_base: next_port_block(&used),
        host_key: host_key.to_owned(),
        added_at,
    })
}

/// A port block chosen by the user: 18000…64900, a multiple of 100, not
/// used by another server.
pub fn check_port_base(cfg: &AppConfig, id: &str, base: u16) -> Result<(), String> {
    if !(18000..=64900).contains(&base) || base % 100 != 0 {
        return Err(lf!("port bloğu 18000–64900 arasında ve 100'ün katı olmalı", "the port block must be 18000–64900 and a multiple of 100"));
    }
    if cfg.servers.iter().any(|s| s.id != id && s.port_base == base) {
        return Err(lf!("{base} başka bir sunucuda kullanılıyor", "{base} is used by another server"));
    }
    Ok(())
}

/// Jobs that run on a server instead of this computer.
pub const SERVER_JOBS: &[&str] = &["server_install", "server_update", "start", "stop", "restart", "up", "build", "server_reset", "server_uninstall"];

/// Which server a job runs on: `explicit` ("local" or a server id, from the
/// UI) wins over the active target; only server job kinds go to a server.
pub fn job_target<'a>(cfg: &'a AppConfig, kind: &str, explicit: Option<&str>) -> Option<&'a ServerConfig> {
    if !SERVER_JOBS.contains(&kind) {
        return None;
    }
    match explicit {
        Some("local") => None,
        Some(id) => find(cfg, id),
        None => find(cfg, &cfg.active_target),
    }
}

/// How ssh names this host in known_hosts: `host`, or `[host]:port`.
pub fn known_host_field(t: &SshTarget) -> String {
    match t.port {
        Some(p) if p != 22 => format!("[{}]:{p}", t.host),
        _ => t.host.clone(),
    }
}

/// Drop the known_hosts lines of `host_field` (a server leaving the list).
pub fn forget_host_key(opts: &SshOpts, host_field: &str) {
    let path = opts.known_hosts();
    if let Ok(old) = std::fs::read_to_string(&path) {
        let out: String = old.lines().filter(|l| l.split_whitespace().next() != Some(host_field) && !l.trim().is_empty()).map(|l| format!("{l}\n")).collect();
        let _ = std::fs::write(&path, out);
    }
}

/// A key file the user named (e.g. a provider's .pem): `~` expanded, must be
/// a file, and made 600 (ssh refuses a private key others can read).
pub fn prepare_identity(path: &str) -> Result<PathBuf, String> {
    let path = path.trim();
    let p = match path.strip_prefix("~/") {
        Some(rest) => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(rest),
        None => PathBuf::from(path),
    };
    if !p.is_file() {
        return Err(lf!("anahtar dosyası bulunamadı: {}", "key file not found: {}", p.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).map(|m| m.permissions().mode() & 0o777).unwrap_or(0o600);
        if mode & 0o077 != 0 {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).map_err(|e| lf!("{} izinleri ayarlanamadı: {e}", "could not set the permissions of {}: {e}", p.display()))?;
        }
    }
    Ok(p)
}

/// Run `script` on `s` with its shell: PowerShell on a Windows server, bash
/// otherwise (also for an unknown mode, as before).
pub fn exec(s: &ServerConfig, o: &SshOpts, script: &str) -> Result<Cmd, String> {
    let shell = ServerMode::parse(&s.mode).map(|m| m.shell()).unwrap_or(Shell::Bash);
    Ok(exec_shell(&s.target()?, o, shell, script))
}

/// What the SSH banner tells about the server: "windows" (Microsoft's
/// OpenSSH port), "linux" (any other SSH server), None (no banner).
pub fn os_from_banner(b: Option<&str>) -> Option<&'static str> {
    let b = b?.trim();
    if !b.starts_with("SSH-") {
        None
    } else if is_windows_banner(b) {
        Some("windows")
    } else {
        Some("linux")
    }
}

/// `uname -s` output (plus the WSL marker line) → "linux", else None (WSL
/// on a Windows host, Git Bash's MINGW64, macOS, nothing).
pub fn uname_os(out: &str) -> Option<&'static str> {
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    (lines.contains(&"Linux") && !lines.contains(&"wsl")).then_some("linux")
}

/// The saved server kind does not match the detected OS.
pub fn os_mismatch(saved_mode: &str, detected: Option<&str>) -> bool {
    match detected {
        Some("windows") => saved_mode != "windows",
        Some("linux") => saved_mode == "windows",
        _ => false,
    }
}

/// The app's ssh state: `<config_dir>/ssh` and a short ControlMaster dir.
pub fn ssh_opts(config_dir: &Path, s: &ServerConfig) -> SshOpts {
    SshOpts { dir: config_dir.join("ssh"), control_dir: control_dir(config_dir), identity: s.identity.clone() }
}

fn control_dir(config_dir: &Path) -> PathBuf {
    #[cfg(unix)]
    let uid = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(config_dir).map(|m| m.uid()).unwrap_or(0)
    };
    #[cfg(not(unix))]
    let uid = {
        let _ = config_dir;
        0
    };
    PathBuf::from(format!("/tmp/smt5-ssh-{uid}"))
}

/// Accept the offered host key with fingerprint `sha256`: its line replaces
/// any earlier lines for that host in the app's known_hosts (mode 600).
pub fn remember_host_key(opts: &SshOpts, keys: &[HostKey], sha256: &str) -> Result<(), String> {
    let key = keys.iter().find(|k| k.sha256 == sha256).ok_or_else(|| lf!("bu parmak izi sunucunun sunduğu anahtarlarda yok", "this fingerprint is not one the server offered"))?;
    let host = key.line.split_whitespace().next().unwrap_or_default();
    let path = opts.known_hosts();
    std::fs::create_dir_all(&opts.dir).map_err(|e| e.to_string())?;
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out: String = old.lines().filter(|l| l.split_whitespace().next() != Some(host) && !l.trim().is_empty()).map(|l| format!("{l}\n")).collect();
    out.push_str(key.line.trim());
    out.push('\n');
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::AppConfig;
    use smt5_core::remote::HostKey;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("smt5-srv-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn config_roundtrip_with_servers() {
        let old: AppConfig = serde_json::from_str(r#"{"lang":"tr","setup_complete":true}"#).unwrap();
        assert!(old.servers.is_empty());
        assert_eq!(old.active_target, "local");
        assert_eq!(AppConfig::default().active_target, "local");
        let mut cfg = old.clone();
        let s = new_server(&cfg, "VPS-1", "ubuntu@203.0.113.10", None, "SHA256:x").unwrap();
        cfg.servers.push(s.clone());
        cfg.active_target = s.id.clone();
        let back: AppConfig = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.servers, vec![s]);
        assert_eq!(back.active_target, cfg.active_target);
    }

    #[test]
    fn new_server_allocates_blocks() {
        let mut cfg = AppConfig::default();
        let a = new_server(&cfg, "A", "u@a", None, "SHA256:a").unwrap();
        assert_eq!((a.port_base, a.mode.as_str(), a.id.len()), (18000, "linux-native", 6));
        cfg.servers.push(a.clone());
        let b = new_server(&cfg, "B", "u@b:2222", Some(PathBuf::from("/k")), "SHA256:b").unwrap();
        assert_eq!(b.port_base, 18100);
        assert_ne!(a.id, b.id);
        assert!(new_server(&cfg, "A2", "u@a", None, "SHA256:a").is_err(), "same ssh twice");
        assert!(new_server(&cfg, "X", "a b", None, "SHA256:x").is_err());
        assert!(new_server(&cfg, "", "u@c", None, "SHA256:x").is_err());
        assert!(new_server(&cfg, &"n".repeat(41), "u@c", None, "SHA256:x").is_err());
        assert_eq!(find(&cfg, &a.id).map(|s| s.name.as_str()), Some("A"));
        let o = ssh_opts(Path::new("/cfg"), &b);
        assert_eq!((o.dir.as_path(), o.identity.as_deref()), (Path::new("/cfg/ssh"), Some(Path::new("/k"))));
        assert!(o.control_dir.display().to_string().len() < 40, "socket dir must stay short");
    }

    #[test]
    fn job_target_rules() {
        let mut cfg = AppConfig::default();
        let a = new_server(&cfg, "A", "u@a", None, "x").unwrap();
        cfg.servers.push(a.clone());
        assert!(job_target(&cfg, "stop", None).is_none(), "local active");
        cfg.active_target = a.id.clone();
        assert_eq!(job_target(&cfg, "stop", None).map(|s| s.id.as_str()), Some(a.id.as_str()));
        assert!(job_target(&cfg, "stop", Some("local")).is_none(), "full quit stops only this computer (I4)");
        assert!(job_target(&cfg, "vm_stop", None).is_none(), "local-only kinds stay local");
        assert_eq!(job_target(&cfg, "up", Some(&a.id)).map(|s| s.id.as_str()), Some(a.id.as_str()));
        cfg.active_target = "local".into();
        assert_eq!(job_target(&cfg, "server_uninstall", Some(&a.id)).map(|s| s.name.as_str()), Some("A"));
        assert_eq!(job_target(&cfg, "server_reset", Some(&a.id)).map(|s| s.name.as_str()), Some("A"));
        assert!(job_target(&cfg, "reset", Some(&a.id)).is_none(), "the local reset never goes to a server");
    }

    #[test]
    fn forget_removes_host_lines() {
        let d = tmpdir("forget");
        let o = SshOpts { dir: d.clone(), control_dir: d.clone(), identity: None };
        std::fs::write(o.known_hosts(), "[h]:2222 ssh-ed25519 AAA\nother ssh-ed25519 BBB\n").unwrap();
        forget_host_key(&o, "[h]:2222");
        assert_eq!(std::fs::read_to_string(o.known_hosts()).unwrap(), "other ssh-ed25519 BBB\n");
        assert_eq!(known_host_field(&SshTarget::parse("u@h:2222").unwrap()), "[h]:2222");
        assert_eq!(known_host_field(&SshTarget::parse("u@h").unwrap()), "h");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn identity_file_is_checked_and_protected() {
        let d = tmpdir("pem");
        assert!(prepare_identity(&d.join("nope.pem").display().to_string()).is_err(), "missing file");
        let f = d.join("my key.pem");
        std::fs::write(&f, "-----BEGIN OPENSSH PRIVATE KEY-----\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(prepare_identity(&f.display().to_string()).unwrap(), f);
            // ssh refuses a key others can read; make it 600
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let old_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &d);
        let tilde = prepare_identity("~/my key.pem");
        if let Some(h) = old_home {
            std::env::set_var("HOME", h);
        }
        assert_eq!(tilde.unwrap(), f, "~ is the home folder");
        assert!(prepare_identity(&d.display().to_string()).is_err(), "a folder is not a key");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn servers_exec_picks_shell() {
        let o = SshOpts { dir: PathBuf::from("/cfg/ssh"), control_dir: PathBuf::from("/tmp/s"), identity: None };
        let mk = |mode: &str| ServerConfig { id: "a".into(), ssh: "u@203.0.113.10".into(), mode: mode.into(), ..Default::default() };
        let w = exec(&mk("windows"), &o, "Write-Output hi").unwrap();
        assert_eq!(w.stdin_text.as_deref(), Some("Write-Output hi"));
        assert!(w.args.iter().any(|a| a.starts_with("powershell ")) && !w.args.iter().any(|a| a == "bash"), "{:?}", w.args);
        for m in ["linux-native", "docker"] {
            let l = exec(&mk(m), &o, "echo hi").unwrap();
            assert!(l.stdin_text.is_none());
            assert!(l.display().ends_with(" bash -lc 'echo hi'"), "{}", l.display());
            assert_eq!(l, smt5_core::remote::exec_cmd(&SshTarget::parse("u@203.0.113.10").unwrap(), &o, "echo hi"), "Linux unchanged");
        }
        assert!(exec(&ServerConfig { ssh: "a b".into(), ..mk("windows") }, &o, "x").is_err());
    }

    #[test]
    fn server_test_os_mismatch() {
        assert!(os_mismatch("windows", Some("linux")));
        assert!(os_mismatch("linux-native", Some("windows")));
        assert!(os_mismatch("docker", Some("windows")));
        assert!(!os_mismatch("docker", Some("linux")));
        assert!(!os_mismatch("windows", Some("windows")));
        assert!(!os_mismatch("windows", None) && !os_mismatch("linux-native", None));
    }

    #[test]
    fn uname_output() {
        assert_eq!(uname_os("Linux\n"), Some("linux"));
        assert_eq!(uname_os("motd line\nLinux\n"), Some("linux"), "any line, not only the first");
        assert_eq!(uname_os("Linux\nwsl\n"), None, "WSL on a Windows host");
        assert_eq!(uname_os("MINGW64_NT-10.0-20348\n"), None, "Git Bash on Windows");
        assert_eq!(uname_os("Darwin\n"), None);
        assert_eq!(uname_os(""), None);
    }

    #[test]
    fn probe_os_from_banner() {
        assert_eq!(os_from_banner(Some("SSH-2.0-OpenSSH_for_Windows_9.5")), Some("windows"));
        assert_eq!(os_from_banner(Some("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5")), Some("linux"));
        assert_eq!(os_from_banner(Some("garbage")), None);
        assert_eq!(os_from_banner(None), None);
    }

    #[test]
    fn port_base_rules() {
        let mut cfg = AppConfig::default();
        let a = new_server(&cfg, "A", "u@a", None, "x").unwrap();
        cfg.servers.push(a.clone());
        let b = new_server(&cfg, "B", "u@b", None, "x").unwrap();
        cfg.servers.push(b.clone());
        assert!(check_port_base(&cfg, &b.id, 18300).is_ok());
        assert!(check_port_base(&cfg, &b.id, 18000).is_err(), "taken by A");
        assert!(check_port_base(&cfg, &a.id, 18000).is_ok(), "its own block");
        assert!(check_port_base(&cfg, &b.id, 18350).is_err(), "multiple of 100");
        assert!(check_port_base(&cfg, &b.id, 17900).is_err());
        assert!(check_port_base(&cfg, &b.id, 65000).is_err());
    }

    #[test]
    fn known_hosts_replaces_host_lines() {
        let d = tmpdir("kh");
        let o = SshOpts { dir: d.clone(), control_dir: d.clone(), identity: None };
        let k = |sha: &str, line: &str| HostKey { kind: "ED25519".into(), sha256: sha.into(), line: line.into() };
        let other = vec![k("SHA256:o", "[other]:22 ssh-ed25519 OOO")];
        remember_host_key(&o, &other, "SHA256:o").unwrap();
        let keys1 = vec![k("SHA256:1", "203.0.113.10 ssh-ed25519 AAA"), k("SHA256:r", "203.0.113.10 ssh-rsa RRR")];
        remember_host_key(&o, &keys1, "SHA256:1").unwrap();
        let keys2 = vec![k("SHA256:2", "203.0.113.10 ssh-ed25519 BBB")];
        remember_host_key(&o, &keys2, "SHA256:2").unwrap();
        let kh = std::fs::read_to_string(o.known_hosts()).unwrap();
        assert_eq!(kh, "[other]:22 ssh-ed25519 OOO\n203.0.113.10 ssh-ed25519 BBB\n");
        assert!(remember_host_key(&o, &keys2, "SHA256:nope").is_err(), "only an offered key can be accepted");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(o.known_hosts()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(d);
    }
}
